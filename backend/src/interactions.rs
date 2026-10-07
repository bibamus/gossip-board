use actix_web::{delete, get, post, put, web, HttpRequest, HttpResponse};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    auth::authenticated_user_id,
    db::DbPool,
    models::{Comment, NewComment, NewVote},
    posts::visible_posts,
    schema::{comments, posts, users, votes},
    tags::TaggedPost,
};

#[derive(Serialize)]
pub(crate) struct VoteSummary {
    score: i64,
    your_vote: Option<i16>,
}

#[derive(Serialize)]
pub(crate) struct PostDetail {
    #[serde(flatten)]
    pub post: TaggedPost,
    #[serde(flatten)]
    pub votes: VoteSummary,
    pub comment_count: i64,
}

#[derive(Serialize)]
struct PostComment {
    #[serde(flatten)]
    comment: Comment,
    username: String,
}

#[derive(Deserialize)]
pub struct CommentInput {
    body: String,
}

#[derive(Deserialize)]
pub struct VoteInput {
    value: i16,
}

fn validate_comment(body: &str) -> Result<&str, &'static str> {
    let body = body.trim();
    if body.is_empty() {
        return Err("Comment cannot be empty.");
    }
    Ok(body)
}

fn validate_vote(value: i16) -> Result<(), &'static str> {
    if !matches!(value, -1 | 1) {
        return Err("Vote must be 1 (upvote) or -1 (downvote).");
    }
    Ok(())
}

fn require_access(connection: &mut PgConnection, post_id: i64, user_id: i64) -> QueryResult<()> {
    visible_posts(user_id)
        .filter(posts::id.eq(post_id))
        .select(posts::id)
        .first::<i64>(connection)?;
    Ok(())
}

pub(crate) fn vote_summary(
    connection: &mut PgConnection,
    post_id: i64,
    user_id: i64,
) -> QueryResult<VoteSummary> {
    let score = votes::table
        .filter(votes::post_id.eq(post_id))
        .select(diesel::dsl::sum(votes::value))
        .first::<Option<i64>>(connection)?
        .unwrap_or(0);
    let your_vote = votes::table
        .filter(votes::post_id.eq(post_id).and(votes::user_id.eq(user_id)))
        .select(votes::value)
        .first::<i16>(connection)
        .optional()?;
    Ok(VoteSummary { score, your_vote })
}

fn load_comments(
    connection: &mut PgConnection,
    post_id: i64,
    user_id: i64,
) -> QueryResult<Vec<PostComment>> {
    require_access(connection, post_id, user_id)?;
    comments::table
        .inner_join(users::table)
        .filter(comments::post_id.eq(post_id))
        .order((comments::created_at.asc(), comments::id.asc()))
        .select((Comment::as_select(), users::username))
        .load::<(Comment, String)>(connection)
        .map(|rows| {
            rows.into_iter()
                .map(|(comment, username)| PostComment { comment, username })
                .collect()
        })
}

fn insert_comment(
    connection: &mut PgConnection,
    post_id: i64,
    user_id: i64,
    body: &str,
) -> QueryResult<PostComment> {
    connection.transaction(|connection| {
        require_access(connection, post_id, user_id)?;
        let comment = diesel::insert_into(comments::table)
            .values(NewComment {
                post_id,
                user_id,
                body,
            })
            .returning(Comment::as_returning())
            .get_result::<Comment>(connection)?;
        let username = users::table
            .find(user_id)
            .select(users::username)
            .first::<String>(connection)?;
        Ok(PostComment { comment, username })
    })
}

fn save_vote(
    connection: &mut PgConnection,
    post_id: i64,
    user_id: i64,
    value: i16,
) -> QueryResult<VoteSummary> {
    connection.transaction(|connection| {
        require_access(connection, post_id, user_id)?;
        diesel::insert_into(votes::table)
            .values(NewVote {
                post_id,
                user_id,
                value,
            })
            .on_conflict((votes::post_id, votes::user_id))
            .do_update()
            .set(votes::value.eq(value))
            .execute(connection)?;
        vote_summary(connection, post_id, user_id)
    })
}

fn remove_vote(
    connection: &mut PgConnection,
    post_id: i64,
    user_id: i64,
) -> QueryResult<VoteSummary> {
    connection.transaction(|connection| {
        require_access(connection, post_id, user_id)?;
        diesel::delete(
            votes::table.filter(votes::post_id.eq(post_id).and(votes::user_id.eq(user_id))),
        )
        .execute(connection)?;
        vote_summary(connection, post_id, user_id)
    })
}

async fn run_query<T, F>(pool: web::Data<DbPool>, query: F) -> actix_web::Result<Option<T>>
where
    T: Send + 'static,
    F: FnOnce(&mut PgConnection) -> QueryResult<T> + Send + 'static,
{
    let result = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        query(&mut connection)
            .optional()
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;
    Ok(result)
}

fn response<T: Serialize>(result: Option<T>, created: bool) -> HttpResponse {
    match result {
        Some(value) if created => HttpResponse::Created().json(value),
        Some(value) => HttpResponse::Ok().json(value),
        None => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Post not found."
        })),
    }
}

#[get("/api/posts/{id}/comments")]
pub async fn get_comments(
    pool: web::Data<DbPool>,
    request: HttpRequest,
    post_id: web::Path<i64>,
) -> actix_web::Result<HttpResponse> {
    let Some(user_id) = authenticated_user_id(pool.clone(), &request).await? else {
        return Ok(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Sign in to read comments."
        })));
    };
    let post_id = post_id.into_inner();
    let result = run_query(pool, move |connection| {
        load_comments(connection, post_id, user_id)
    })
    .await?;
    Ok(response(result, false))
}

#[post("/api/posts/{id}/comments")]
pub async fn create_comment(
    pool: web::Data<DbPool>,
    request: HttpRequest,
    post_id: web::Path<i64>,
    input: web::Json<CommentInput>,
) -> actix_web::Result<HttpResponse> {
    let Some(user_id) = authenticated_user_id(pool.clone(), &request).await? else {
        return Ok(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Sign in to comment."
        })));
    };
    let body = match validate_comment(&input.body) {
        Ok(body) => body.to_owned(),
        Err(error) => {
            return Ok(HttpResponse::BadRequest().json(serde_json::json!({ "error": error })));
        }
    };
    let post_id = post_id.into_inner();
    let result = run_query(pool, move |connection| {
        insert_comment(connection, post_id, user_id, &body)
    })
    .await?;
    Ok(response(result, true))
}

async fn cast_vote(
    pool: web::Data<DbPool>,
    request: HttpRequest,
    post_id: web::Path<i64>,
    input: web::Json<VoteInput>,
) -> actix_web::Result<HttpResponse> {
    let Some(user_id) = authenticated_user_id(pool.clone(), &request).await? else {
        return Ok(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Sign in to vote."
        })));
    };
    if let Err(error) = validate_vote(input.value) {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({ "error": error })));
    }
    let post_id = post_id.into_inner();
    let value = input.value;
    let result = run_query(pool, move |connection| {
        save_vote(connection, post_id, user_id, value)
    })
    .await?;
    Ok(response(result, false))
}

#[post("/api/posts/{id}/votes")]
pub async fn create_vote(
    pool: web::Data<DbPool>,
    request: HttpRequest,
    post_id: web::Path<i64>,
    input: web::Json<VoteInput>,
) -> actix_web::Result<HttpResponse> {
    cast_vote(pool, request, post_id, input).await
}

#[put("/api/posts/{id}/votes")]
pub async fn update_vote(
    pool: web::Data<DbPool>,
    request: HttpRequest,
    post_id: web::Path<i64>,
    input: web::Json<VoteInput>,
) -> actix_web::Result<HttpResponse> {
    cast_vote(pool, request, post_id, input).await
}

#[delete("/api/posts/{id}/votes")]
pub async fn delete_vote(
    pool: web::Data<DbPool>,
    request: HttpRequest,
    post_id: web::Path<i64>,
) -> actix_web::Result<HttpResponse> {
    let Some(user_id) = authenticated_user_id(pool.clone(), &request).await? else {
        return Ok(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Sign in to remove your vote."
        })));
    };
    let post_id = post_id.into_inner();
    let result = run_query(pool, move |connection| {
        remove_vote(connection, post_id, user_id)
    })
    .await?;
    Ok(response(result, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn validates_comments_and_votes() {
        assert_eq!(validate_comment("  A comment\n"), Ok("A comment"));
        assert!(validate_comment(" \n\t").is_err());
        assert!(validate_vote(1).is_ok());
        assert!(validate_vote(-1).is_ok());
        for value in [0, 2, -2, i16::MAX] {
            assert!(validate_vote(value).is_err());
        }
    }

    #[actix_web::test]
    async fn interaction_routes_reject_unauthenticated_requests() {
        use actix_web::{http::StatusCode, test, App};
        use diesel::r2d2::{ConnectionManager, Pool};

        let pool =
            Pool::builder()
                .max_size(1)
                .build_unchecked(ConnectionManager::<PgConnection>::new(
                    "postgres://localhost/unused",
                ));
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(pool))
                .service(get_comments)
                .service(create_comment)
                .service(create_vote)
                .service(update_vote)
                .service(delete_vote)
                .service(crate::posts::get_post),
        )
        .await;
        for uri in ["/api/posts/1/comments", "/api/posts/1"] {
            let request = test::TestRequest::get().uri(uri).to_request();
            let response = test::call_service(&app, request).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        let requests = [
            test::TestRequest::delete()
                .uri("/api/posts/1/votes")
                .to_request(),
            test::TestRequest::post()
                .uri("/api/posts/1/comments")
                .set_json(serde_json::json!({ "body": "A comment" }))
                .to_request(),
            test::TestRequest::post()
                .uri("/api/posts/1/votes")
                .set_json(serde_json::json!({ "value": 1 }))
                .to_request(),
            test::TestRequest::put()
                .uri("/api/posts/1/votes")
                .set_json(serde_json::json!({ "value": -1 }))
                .to_request(),
        ];
        for request in requests {
            let response = test::call_service(&app, request).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }

    #[test]
    #[ignore = "requires TEST_DATABASE_URL; all fixtures are rolled back"]
    fn database_interactions_enforce_access_and_replace_votes() {
        let url = std::env::var("TEST_DATABASE_URL").expect("set TEST_DATABASE_URL");
        let mut connection = PgConnection::establish(&url).expect("connect to test database");
        connection.test_transaction::<(), diesel::result::Error, _>(|connection| {
            let suffix = Utc::now();
            let suffix = suffix.timestamp_nanos_opt().unwrap().to_string();
            let mut ids = Vec::new();
            for name in ["owner", "recipient", "outsider"] {
                ids.push(
                    diesel::insert_into(users::table)
                        .values((
                            users::email.eq(format!("{name}-{suffix}@example.com")),
                            users::username.eq(format!("{name}-{suffix}")),
                        ))
                        .returning(users::id)
                        .get_result::<i64>(connection)?,
                );
            }
            let (owner, recipient, outsider) = (ids[0], ids[1], ids[2]);
            let post_id = diesel::insert_into(posts::table)
                .values(crate::models::NewPost {
                    author_id: owner,
                    title: "Interaction fixture",
                    body: "Content",
                    image_data: None,
                })
                .returning(posts::id)
                .get_result::<i64>(connection)?;
            diesel::insert_into(crate::schema::post_shares::table)
                .values(crate::models::NewPostShare {
                    post_id,
                    shared_by_user_id: owner,
                    shared_with_user_id: recipient,
                })
                .execute(connection)?;

            assert!(load_comments(connection, post_id, owner)?.is_empty());
            let summary = vote_summary(connection, post_id, owner)?;
            assert_eq!(summary.score, 0);
            assert_eq!(summary.your_vote, None);
            let first = insert_comment(connection, post_id, owner, "First")?;
            let second = insert_comment(connection, post_id, recipient, "Second")?;
            let comments = load_comments(connection, post_id, recipient)?;
            assert_eq!(comments.len(), 2);
            assert_eq!(comments[0].comment.id, first.comment.id);
            assert_eq!(comments[1].comment.id, second.comment.id);
            let comment_json = serde_json::to_value(&comments[0]).unwrap();
            assert!(comment_json.get("username").is_some());
            assert!(comment_json.get("email").is_none());
            assert_eq!(save_vote(connection, post_id, owner, 1)?.score, 1);
            assert_eq!(save_vote(connection, post_id, owner, 1)?.score, 1);
            assert_eq!(save_vote(connection, post_id, recipient, 1)?.score, 2);
            let summary = save_vote(connection, post_id, owner, -1)?;
            assert_eq!(summary.score, 0);
            assert_eq!(summary.your_vote, Some(-1));
            assert_eq!(
                votes::table
                    .filter(votes::post_id.eq(post_id))
                    .count()
                    .get_result::<i64>(connection)?,
                2
            );
            assert!(matches!(
                load_comments(connection, post_id, outsider),
                Err(diesel::result::Error::NotFound)
            ));
            assert!(matches!(
                insert_comment(connection, post_id, outsider, "Denied"),
                Err(diesel::result::Error::NotFound)
            ));
            assert!(matches!(
                save_vote(connection, post_id, outsider, 1),
                Err(diesel::result::Error::NotFound)
            ));
            assert!(matches!(
                load_comments(connection, -1, owner),
                Err(diesel::result::Error::NotFound)
            ));
            assert_eq!(load_comments(connection, post_id, owner)?.len(), 2);
            assert_eq!(vote_summary(connection, post_id, owner)?.score, 0);
            assert!(matches!(
                remove_vote(connection, post_id, outsider),
                Err(diesel::result::Error::NotFound)
            ));
            assert!(matches!(
                remove_vote(connection, -1, owner),
                Err(diesel::result::Error::NotFound)
            ));
            let summary = remove_vote(connection, post_id, owner)?;
            assert_eq!(summary.score, 1);
            assert_eq!(summary.your_vote, None);
            assert_eq!(remove_vote(connection, post_id, owner)?.score, 1);
            assert_eq!(
                vote_summary(connection, post_id, recipient)?.your_vote,
                Some(1)
            );
            assert_eq!(save_vote(connection, post_id, owner, 1)?.score, 2);
            let summary = remove_vote(connection, post_id, owner)?;
            assert_eq!(summary.score, 1);
            assert_eq!(summary.your_vote, None);
            let summary = remove_vote(connection, post_id, recipient)?;
            assert_eq!(summary.score, 0);
            assert_eq!(summary.your_vote, None);
            assert_eq!(
                votes::table
                    .filter(votes::post_id.eq(post_id))
                    .count()
                    .get_result::<i64>(connection)?,
                0
            );
            Ok(())
        });
    }
}
