use actix_web::{delete, get, post, put, web, HttpRequest, HttpResponse};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    auth::authenticated_user_id,
    db::DbPool,
    interactions::{vote_summary, PostDetail},
    models::{NewPost, NewPostShare, Post, PostShare},
    schema::{post_shares, posts, users},
};

pub(crate) fn visible_posts(user_id: i64) -> posts::BoxedQuery<'static, diesel::pg::Pg> {
    let shared_post_ids = post_shares::table
        .filter(post_shares::shared_with_user_id.eq(user_id))
        .select(post_shares::post_id);
    posts::table
        .filter(
            posts::author_id
                .eq(user_id)
                .or(posts::id.eq_any(shared_post_ids)),
        )
        .into_boxed()
}

#[derive(Deserialize)]
pub struct PostInput {
    title: String,
    body: String,
}

#[derive(Deserialize)]
pub struct ShareInput {
    username: String,
}

#[derive(Deserialize)]
pub struct UserSearchQuery {
    q: String,
}

#[derive(Serialize)]
struct UserSuggestion {
    id: i64,
    username: String,
}

#[derive(Serialize)]
struct ShareRecipient {
    id: i64,
    post_id: i64,
    shared_by_user_id: i64,
    shared_with_user_id: i64,
    username: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug)]
enum ShareError {
    NotFound,
    SelfShare,
    AlreadyShared,
    Database(diesel::result::Error),
}

impl From<diesel::result::Error> for ShareError {
    fn from(error: diesel::result::Error) -> Self {
        Self::Database(error)
    }
}

fn normalize_username(value: &str) -> Result<String, &'static str> {
    let username = value.trim().trim_start_matches('@').to_ascii_lowercase();
    if username.is_empty()
        || username.len() > 50
        || !username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("Enter a valid username.");
    }
    Ok(username)
}

fn validate_post(input: &PostInput) -> Result<(), &'static str> {
    let title = input.title.trim();
    if title.is_empty() || title.chars().count() > 200 {
        return Err("Title must contain between 1 and 200 characters.");
    }
    if input.body.trim().is_empty() {
        return Err("Post content cannot be empty.");
    }
    Ok(())
}

enum PostUpdate {
    Updated(Post),
    Shared,
    NotFound,
}

fn edit_owned_post(
    connection: &mut PgConnection,
    post_id: i64,
    user_id: i64,
    title: &str,
    body: &str,
) -> QueryResult<PostUpdate> {
    connection.transaction(|connection| {
        let owned_post = posts::table
            .filter(posts::id.eq(post_id).and(posts::author_id.eq(user_id)))
            .select(posts::id)
            .for_update()
            .first::<i64>(connection)
            .optional()?;
        if owned_post.is_none() {
            return Ok(PostUpdate::NotFound);
        }
        let shared = diesel::select(diesel::dsl::exists(
            post_shares::table.filter(post_shares::post_id.eq(post_id)),
        ))
        .get_result::<bool>(connection)?;
        if shared {
            return Ok(PostUpdate::Shared);
        }
        let post = diesel::update(posts::table.find(post_id))
            .set((
                posts::title.eq(title),
                posts::body.eq(body),
                posts::updated_at.eq(Utc::now()),
            ))
            .returning(Post::as_returning())
            .get_result::<Post>(connection)?;
        Ok(PostUpdate::Updated(post))
    })
}

#[get("/api/posts/{id}/shares")]
pub async fn list_shares(
    pool: web::Data<DbPool>,
    request: HttpRequest,
    post_id: web::Path<i64>,
) -> actix_web::Result<HttpResponse> {
    let Some(user_id) = authenticated_user_id(pool.clone(), &request).await? else {
        return Ok(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Sign in to view post shares."
        })));
    };
    let post_id = post_id.into_inner();

    let result = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        let post_exists = visible_posts(user_id)
            .filter(posts::id.eq(post_id))
            .select(posts::id)
            .first::<i64>(&mut connection)
            .optional()
            .map_err(|error| error.to_string())?;
        if post_exists.is_none() {
            return Ok(None);
        }

        let recipients = post_shares::table
            .inner_join(users::table.on(users::id.eq(post_shares::shared_with_user_id)))
            .filter(post_shares::post_id.eq(post_id))
            .order(post_shares::created_at.asc())
            .select((
                post_shares::id,
                post_shares::post_id,
                post_shares::shared_by_user_id,
                post_shares::shared_with_user_id,
                users::username,
                post_shares::created_at,
            ))
            .load::<(i64, i64, i64, i64, String, DateTime<Utc>)>(&mut connection)
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(
                |(id, post_id, shared_by_user_id, shared_with_user_id, username, created_at)| {
                    ShareRecipient {
                        id,
                        post_id,
                        shared_by_user_id,
                        shared_with_user_id,
                        username,
                        created_at,
                    }
                },
            )
            .collect::<Vec<_>>();
        Ok::<_, String>(Some(recipients))
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;

    match result {
        Some(recipients) => Ok(HttpResponse::Ok().json(recipients)),
        None => Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Post not found."
        }))),
    }
}

#[get("/api/users/search")]
pub async fn search_users(
    pool: web::Data<DbPool>,
    request: HttpRequest,
    query: web::Query<UserSearchQuery>,
) -> actix_web::Result<HttpResponse> {
    let Some(user_id) = authenticated_user_id(pool.clone(), &request).await? else {
        return Ok(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Sign in to search for people."
        })));
    };
    let query = query.q.trim().trim_start_matches('@').to_lowercase();
    if query.chars().count() < 2 {
        return Ok(HttpResponse::Ok().json(Vec::<UserSuggestion>::new()));
    }
    if query.chars().count() > 50 {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Search text is too long."
        })));
    }
    let escaped_query = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let pattern = format!("%{escaped_query}%");

    let suggestions = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        users::table
            .filter(users::id.ne(user_id))
            .filter(users::username.ilike(&pattern))
            .order(users::username.asc())
            .limit(10)
            .select((users::id, users::username))
            .load::<(i64, String)>(&mut connection)
            .map(|rows| {
                rows.into_iter()
                    .map(|(id, username)| UserSuggestion { id, username })
                    .collect::<Vec<_>>()
            })
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;

    Ok(HttpResponse::Ok().json(suggestions))
}

#[post("/api/posts/{id}/shares")]
pub async fn share_post(
    pool: web::Data<DbPool>,
    request: HttpRequest,
    post_id: web::Path<i64>,
    input: web::Json<ShareInput>,
) -> actix_web::Result<HttpResponse> {
    let Some(user_id) = authenticated_user_id(pool.clone(), &request).await? else {
        return Ok(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Sign in to share posts."
        })));
    };
    let username = match normalize_username(&input.username) {
        Ok(username) => username,
        Err(error) => {
            return Ok(HttpResponse::BadRequest().json(serde_json::json!({ "error": error })));
        }
    };
    let post_id = post_id.into_inner();

    let outcome = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        let result = connection.transaction::<ShareRecipient, ShareError, _>(|connection| {
            let has_access = visible_posts(user_id)
                .filter(posts::id.eq(post_id))
                .select(posts::id)
                .first::<i64>(connection)
                .optional()?
                .is_some();
            if !has_access {
                return Err(ShareError::NotFound);
            }

            // Serialize sharing with editing so the first share freezes the content.
            posts::table
                .find(post_id)
                .select(posts::id)
                .for_update()
                .first::<i64>(connection)?;

            let recipient_id = users::table
                .filter(users::username.eq(&username))
                .select(users::id)
                .first::<i64>(connection)
                .optional()?
                .ok_or(ShareError::NotFound)?;
            if recipient_id == user_id {
                return Err(ShareError::SelfShare);
            }

            let share = diesel::insert_into(post_shares::table)
                .values(NewPostShare {
                    post_id,
                    shared_by_user_id: user_id,
                    shared_with_user_id: recipient_id,
                })
                .on_conflict((post_shares::post_id, post_shares::shared_with_user_id))
                .do_nothing()
                .returning(PostShare::as_returning())
                .get_result::<PostShare>(connection)
                .optional()?
                .ok_or(ShareError::AlreadyShared)?;

            let recipient_username = users::table
                .find(recipient_id)
                .select(users::username)
                .first::<String>(connection)?;
            Ok(ShareRecipient {
                id: share.id,
                post_id: share.post_id,
                shared_by_user_id: share.shared_by_user_id,
                shared_with_user_id: share.shared_with_user_id,
                username: recipient_username,
                created_at: share.created_at,
            })
        });
        Ok::<_, String>(result)
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?;

    match outcome {
        Err(error) => Err(actix_web::error::ErrorInternalServerError(error)),
        Ok(Err(ShareError::NotFound)) => Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Post or username not found."
        }))),
        Ok(Err(ShareError::SelfShare)) => Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "You cannot share a post with yourself."
        }))),
        Ok(Err(ShareError::AlreadyShared)) => {
            Ok(HttpResponse::Conflict().json(serde_json::json!({
                "error": "This post has already been shared with that user."
            })))
        }
        Ok(Err(ShareError::Database(error))) => {
            Err(actix_web::error::ErrorInternalServerError(error))
        }
        Ok(Ok(share)) => Ok(HttpResponse::Created().json(share)),
    }
}

#[get("/api/posts")]
pub async fn list_posts(
    pool: web::Data<DbPool>,
    request: HttpRequest,
) -> actix_web::Result<HttpResponse> {
    let Some(user_id) = authenticated_user_id(pool.clone(), &request).await? else {
        return Ok(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Sign in to view posts."
        })));
    };

    let result = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        visible_posts(user_id)
            .order((posts::created_at.desc(), posts::id.desc()))
            .select(Post::as_select())
            .load::<Post>(&mut connection)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;

    Ok(HttpResponse::Ok().json(result))
}

#[get("/api/posts/{id}")]
pub async fn get_post(
    pool: web::Data<DbPool>,
    request: HttpRequest,
    post_id: web::Path<i64>,
) -> actix_web::Result<HttpResponse> {
    let Some(user_id) = authenticated_user_id(pool.clone(), &request).await? else {
        return Ok(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Sign in to view posts."
        })));
    };
    let post_id = post_id.into_inner();

    let result = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        connection
            .transaction::<_, diesel::result::Error, _>(|connection| {
                let Some(post) = visible_posts(user_id)
                    .filter(posts::id.eq(post_id))
                    .select(Post::as_select())
                    .first::<Post>(connection)
                    .optional()?
                else {
                    return Ok(None);
                };
                let votes = vote_summary(connection, post_id, user_id)?;
                let comment_count = crate::schema::comments::table
                    .filter(crate::schema::comments::post_id.eq(post_id))
                    .count()
                    .get_result::<i64>(connection)?;
                Ok(Some(PostDetail {
                    post,
                    votes,
                    comment_count,
                }))
            })
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;

    match result {
        Some(post) => Ok(HttpResponse::Ok().json(post)),
        None => Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Post not found."
        }))),
    }
}

#[post("/api/posts")]
pub async fn create_post(
    pool: web::Data<DbPool>,
    request: HttpRequest,
    input: web::Json<PostInput>,
) -> actix_web::Result<HttpResponse> {
    let Some(user_id) = authenticated_user_id(pool.clone(), &request).await? else {
        return Ok(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Sign in to create posts."
        })));
    };
    if let Err(error) = validate_post(&input) {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({ "error": error })));
    }
    let title = input.title.trim().to_owned();
    let body = input.body.trim().to_owned();

    let result = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        diesel::insert_into(posts::table)
            .values(NewPost {
                author_id: user_id,
                title: &title,
                body: &body,
            })
            .returning(Post::as_returning())
            .get_result::<Post>(&mut connection)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;

    Ok(HttpResponse::Created().json(result))
}

#[put("/api/posts/{id}")]
pub async fn update_post(
    pool: web::Data<DbPool>,
    request: HttpRequest,
    post_id: web::Path<i64>,
    input: web::Json<PostInput>,
) -> actix_web::Result<HttpResponse> {
    let Some(user_id) = authenticated_user_id(pool.clone(), &request).await? else {
        return Ok(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Sign in to edit posts."
        })));
    };
    if let Err(error) = validate_post(&input) {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({ "error": error })));
    }
    let post_id = post_id.into_inner();
    let title = input.title.trim().to_owned();
    let body = input.body.trim().to_owned();
    let result = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        edit_owned_post(&mut connection, post_id, user_id, &title, &body)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;

    match result {
        PostUpdate::Updated(post) => Ok(HttpResponse::Ok().json(post)),
        PostUpdate::Shared => Ok(HttpResponse::Conflict().json(serde_json::json!({
            "error": "Shared posts cannot be edited. You can still delete your post."
        }))),
        PostUpdate::NotFound => Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Post not found."
        }))),
    }
}

#[delete("/api/posts/{id}")]
pub async fn delete_post(
    pool: web::Data<DbPool>,
    request: HttpRequest,
    post_id: web::Path<i64>,
) -> actix_web::Result<HttpResponse> {
    let Some(user_id) = authenticated_user_id(pool.clone(), &request).await? else {
        return Ok(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Sign in to delete posts."
        })));
    };
    let post_id = post_id.into_inner();

    let deleted = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        diesel::delete(posts::table.filter(posts::id.eq(post_id).and(posts::author_id.eq(user_id))))
            .execute(&mut connection)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;

    if deleted == 0 {
        return Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Post not found."
        })));
    }
    Ok(HttpResponse::NoContent().finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires TEST_DATABASE_URL; all fixtures are rolled back"]
    fn shared_posts_cannot_be_edited_but_can_be_deleted() {
        let url = std::env::var("TEST_DATABASE_URL").expect("set TEST_DATABASE_URL");
        let mut connection = PgConnection::establish(&url).expect("connect to test database");
        connection.test_transaction::<(), diesel::result::Error, _>(|connection| {
            let suffix = Utc::now().timestamp_nanos_opt().unwrap().to_string();
            let mut ids = Vec::new();
            for name in ["owner", "recipient"] {
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
            let (owner, recipient) = (ids[0], ids[1]);
            let post = diesel::insert_into(posts::table)
                .values(NewPost {
                    author_id: owner,
                    title: "Original",
                    body: "Original body",
                })
                .returning(Post::as_returning())
                .get_result::<Post>(connection)?;
            assert!(matches!(
                edit_owned_post(connection, post.id, recipient, "Denied", "Denied")?,
                PostUpdate::NotFound
            ));
            let updated =
                match edit_owned_post(connection, post.id, owner, "Edited", "Edited body")? {
                    PostUpdate::Updated(post) => post,
                    _ => panic!("unshared post should be editable"),
                };
            diesel::insert_into(post_shares::table)
                .values(NewPostShare {
                    post_id: post.id,
                    shared_by_user_id: owner,
                    shared_with_user_id: recipient,
                })
                .execute(connection)?;
            assert!(matches!(
                edit_owned_post(connection, post.id, owner, "Denied", "Denied")?,
                PostUpdate::Shared
            ));
            let unchanged = posts::table.find(post.id).first::<Post>(connection)?;
            assert_eq!(unchanged.title, updated.title);
            assert_eq!(unchanged.body, updated.body);
            assert_eq!(unchanged.updated_at, updated.updated_at);
            assert!(matches!(
                edit_owned_post(connection, post.id, recipient, "Denied", "Denied")?,
                PostUpdate::NotFound
            ));
            assert_eq!(
                diesel::delete(
                    posts::table.filter(posts::id.eq(post.id).and(posts::author_id.eq(recipient)))
                )
                .execute(connection)?,
                0
            );
            assert_eq!(
                diesel::delete(
                    posts::table.filter(posts::id.eq(post.id).and(posts::author_id.eq(owner)))
                )
                .execute(connection)?,
                1
            );
            assert_eq!(
                post_shares::table
                    .filter(post_shares::post_id.eq(post.id))
                    .count()
                    .get_result::<i64>(connection)?,
                0
            );
            Ok(())
        });
    }

    #[test]
    fn accepts_only_normalized_usernames_for_sharing() {
        assert_eq!(
            normalize_username("  @Gossip-Friend  "),
            Ok("gossip-friend".to_owned())
        );
        assert!(normalize_username("friend@example.com").is_err());
        assert!(normalize_username("invalid recipient").is_err());
    }

    #[test]
    fn validates_post_title_and_body() {
        assert!(validate_post(&PostInput {
            title: "A title".to_owned(),
            body: "A story".to_owned(),
        })
        .is_ok());
        assert!(validate_post(&PostInput {
            title: "  ".to_owned(),
            body: "A story".to_owned(),
        })
        .is_err());
        assert!(validate_post(&PostInput {
            title: "x".repeat(201),
            body: "A story".to_owned(),
        })
        .is_err());
        assert!(validate_post(&PostInput {
            title: "é".repeat(200),
            body: "A story".to_owned(),
        })
        .is_ok());
        assert!(validate_post(&PostInput {
            title: "A title".to_owned(),
            body: "  ".to_owned(),
        })
        .is_err());
    }
}
