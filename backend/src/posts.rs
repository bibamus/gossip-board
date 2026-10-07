use actix_web::{delete, get, post, put, web, HttpRequest, HttpResponse};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    auth::{authenticated_user_id, normalize_username},
    db::DbPool,
    interactions::{vote_summary, PostDetail},
    models::{NewPost, NewPostShare, Post, PostShare},
    schema::{post_shares, posts, users},
    tags::{
        normalize_tag, normalize_tags, replace_post_tags, tagged_post, visible_tagged_posts,
        TaggedPost,
    },
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
    tags: Option<Vec<String>>,
    image_data: Option<String>,
}

#[derive(Deserialize)]
pub struct PostListQuery {
    tag: Option<String>,
}

#[derive(Deserialize)]
pub struct PostTagsInput {
    tags: Vec<String>,
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

fn validate_post(input: &PostInput) -> Result<(), &'static str> {
    let title = input.title.trim();
    if title.is_empty() || title.chars().count() > 200 {
        return Err("Title must contain between 1 and 200 characters.");
    }
    if input.body.trim().is_empty() {
        return Err("Post content cannot be empty.");
    }
    if let Some(image_data) = input.image_data.as_deref().filter(|data| !data.is_empty()) {
        if !valid_image_data(image_data) {
            return Err("Image must be a PNG, JPEG, GIF, or WebP no larger than 3 MB.");
        }
    }
    Ok(())
}

fn valid_image_data(image_data: &str) -> bool {
    let Some((metadata, encoded)) = image_data.split_once(',') else {
        return false;
    };
    if !matches!(
        metadata,
        "data:image/png;base64"
            | "data:image/jpeg;base64"
            | "data:image/gif;base64"
            | "data:image/webp;base64"
    ) || encoded.is_empty()
        || encoded.len() % 4 != 0
    {
        return false;
    }

    let encoded = encoded.as_bytes();
    let padding = encoded
        .iter()
        .rev()
        .take_while(|byte| **byte == b'=')
        .count();
    padding <= 2
        && encoded[..encoded.len() - padding]
            .iter()
            .copied()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'/')
        && encoded.len() / 4 * 3 - padding <= 3 * 1024 * 1024
}

enum PostUpdate {
    Updated(TaggedPost),
    Shared,
    NotFound,
}

fn create_owned_post(
    connection: &mut PgConnection,
    user_id: i64,
    title: &str,
    body: &str,
    image_data: Option<&str>,
    tag_names: &[String],
) -> QueryResult<TaggedPost> {
    connection.transaction(|connection| {
        let post = diesel::insert_into(posts::table)
            .values(NewPost {
                author_id: user_id,
                title,
                body,
                image_data,
            })
            .returning(Post::as_returning())
            .get_result::<Post>(connection)?;
        replace_post_tags(connection, post.id, tag_names)?;
        tagged_post(connection, post)
    })
}

fn edit_owned_post(
    connection: &mut PgConnection,
    post_id: i64,
    user_id: i64,
    content: Option<(&str, &str)>,
    tag_names: Option<&[String]>,
    image_data: Option<&str>,
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
        if shared && content.is_some() {
            return Ok(PostUpdate::Shared);
        }
        if let Some((title, body)) = content {
            diesel::update(posts::table.find(post_id))
                .set((
                    posts::title.eq(title),
                    posts::body.eq(body),
                    posts::updated_at.eq(Utc::now()),
                ))
                .execute(connection)?;
        } else {
            diesel::update(posts::table.find(post_id))
                .set(posts::updated_at.eq(Utc::now()))
                .execute(connection)?;
        }
        if let Some(names) = tag_names {
            replace_post_tags(connection, post_id, names)?;
        }
        if let Some(image_data) = image_data {
            diesel::update(posts::table.find(post_id))
                .set(posts::image_data.eq((!image_data.is_empty()).then_some(image_data)))
                .execute(connection)?;
        }
        let post = posts::table
            .find(post_id)
            .select(Post::as_select())
            .first::<Post>(connection)?;
        Ok(PostUpdate::Updated(tagged_post(connection, post)?))
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
    query: web::Query<PostListQuery>,
) -> actix_web::Result<HttpResponse> {
    let Some(user_id) = authenticated_user_id(pool.clone(), &request).await? else {
        return Ok(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Sign in to view posts."
        })));
    };

    let topic = match query.tag.as_deref().map(normalize_tag).transpose() {
        Ok(topic) => topic,
        Err(error) => {
            return Ok(HttpResponse::BadRequest().json(serde_json::json!({ "error": error })));
        }
    };
    let result = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        visible_tagged_posts(&mut connection, user_id, topic.as_deref())
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
                    post: tagged_post(connection, post)?,
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
    let image_data = input.image_data.as_deref().map(str::to_owned);
    let tag_names = match normalize_tags(input.tags.as_deref().unwrap_or_default()) {
        Ok(names) => names,
        Err(error) => {
            return Ok(HttpResponse::BadRequest().json(serde_json::json!({ "error": error })));
        }
    };

    let result = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        create_owned_post(
            &mut connection,
            user_id,
            &title,
            &body,
            image_data.as_deref().filter(|data| !data.is_empty()),
            &tag_names,
        )
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
    let image_data = input.image_data.as_deref().map(str::to_owned);
    let tag_names = match input.tags.as_deref().map(normalize_tags).transpose() {
        Ok(names) => names,
        Err(error) => {
            return Ok(HttpResponse::BadRequest().json(serde_json::json!({ "error": error })));
        }
    };
    let result = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        edit_owned_post(
            &mut connection,
            post_id,
            user_id,
            Some((&title, &body)),
            tag_names.as_deref(),
            image_data.as_deref(),
        )
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;

    Ok(post_update_response(result))
}

fn post_update_response(result: PostUpdate) -> HttpResponse {
    match result {
        PostUpdate::Updated(post) => HttpResponse::Ok().json(post),
        PostUpdate::Shared => HttpResponse::Conflict().json(serde_json::json!({
            "error": "Shared post content cannot be edited. You can still change topics or delete your post."
        })),
        PostUpdate::NotFound => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Post not found."
        })),
    }
}

#[put("/api/posts/{id}/tags")]
pub async fn update_post_tags(
    pool: web::Data<DbPool>,
    request: HttpRequest,
    post_id: web::Path<i64>,
    input: web::Json<PostTagsInput>,
) -> actix_web::Result<HttpResponse> {
    let Some(user_id) = authenticated_user_id(pool.clone(), &request).await? else {
        return Ok(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Sign in to edit topics."
        })));
    };
    let names = match normalize_tags(&input.tags) {
        Ok(names) => names,
        Err(error) => {
            return Ok(HttpResponse::BadRequest().json(serde_json::json!({ "error": error })));
        }
    };
    let post_id = post_id.into_inner();
    let result = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        edit_owned_post(&mut connection, post_id, user_id, None, Some(&names), None)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;
    Ok(post_update_response(result))
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
    fn topics_are_deduplicated_access_filtered_and_locked_after_sharing() {
        use crate::tags::visible_topics;
        let url = std::env::var("TEST_DATABASE_URL").expect("set TEST_DATABASE_URL");
        let mut connection = PgConnection::establish(&url).expect("connect to test database");
        connection.test_transaction::<(), diesel::result::Error, _>(|connection| {
            let suffix = Utc::now().timestamp_nanos_opt().unwrap().to_string();
            let mut ids = Vec::new();
            for name in ["topicowner", "topicrecipient", "topicoutsider"] {
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
            let names = normalize_tags(&["Office News".to_owned(), "office-news".to_owned(), "Rumors".to_owned()]).unwrap();
            let tagged = create_owned_post(connection, owner, "Tagged", "Content", None, &names)?;
            assert_eq!(tagged.tags.len(), 2);
            assert_eq!(tagged.tags[0].name, "office news");
            assert_eq!(tagged.tags[0].slug, "office-news");
            let mut hidden_names = names.clone();
            hidden_names.push("private only".to_owned());
            let second = create_owned_post(connection, outsider, "Hidden", "Private", None, &hidden_names)?;
            assert_eq!(tagged.tags[0].id, second.tags[0].id);
            assert_eq!(tagged.tags[1].id, second.tags[2].id);
            let untagged = create_owned_post(connection, owner, "Untagged", "Content", None, &[])?;
            assert!(untagged.tags.is_empty());
            assert_eq!(visible_tagged_posts(connection, owner, None)?.len(), 2);
            assert!(!crate::tags::visible_topics(connection, owner)?.iter().any(|tag| tag.name == "private only"));
            let filtered = visible_tagged_posts(connection, owner, Some("office news"))?;
            assert_eq!(filtered.len(), 1);
            assert_eq!(filtered[0].post.id, tagged.post.id);
            assert!(visible_tagged_posts(connection, recipient, Some("office news"))?.is_empty());
            assert!(visible_topics(connection, recipient)?.is_empty());
            assert!(visible_tagged_posts(connection, owner, Some("unknown"))?.is_empty());
            assert!(matches!(
                edit_owned_post(connection, tagged.post.id, recipient, None, Some(&[]), None)?,
                PostUpdate::NotFound
            ));
            let replacements = normalize_tags(&["Relationships".to_owned()]).unwrap();
            let updated = match edit_owned_post(connection, tagged.post.id, owner, None, Some(&replacements), None)? {
                PostUpdate::Updated(post) => post,
                _ => panic!("unshared topics should be editable"),
            };
            assert_eq!(updated.post.title, tagged.post.title);
            assert_eq!(updated.tags[0].name, "relationships");
            assert!(visible_tagged_posts(connection, owner, Some("office news"))?.is_empty());
            assert!(matches!(
                edit_owned_post(connection, tagged.post.id, owner, Some(("Edited", "Content")), None, None)?,
                PostUpdate::Updated(post) if post.tags.len() == 1
            ));
            assert!(matches!(
                edit_owned_post(connection, tagged.post.id, owner, None, Some(&[]), None)?,
                PostUpdate::Updated(post) if post.tags.is_empty()
            ));
            edit_owned_post(connection, tagged.post.id, owner, None, Some(&names), None)?;
            diesel::insert_into(post_shares::table)
                .values(NewPostShare {
                    post_id: tagged.post.id,
                    shared_by_user_id: owner,
                    shared_with_user_id: recipient,
                })
                .execute(connection)?;
            let shared = visible_tagged_posts(connection, recipient, Some("office news"))?;
            assert_eq!(shared.len(), 1);
            assert_eq!(shared[0].post.id, tagged.post.id);
            assert_eq!(visible_topics(connection, recipient)?.len(), 2);
            assert!(matches!(
                edit_owned_post(connection, tagged.post.id, owner, None, Some(&[]), None)?,
                PostUpdate::Updated(post) if post.tags.is_empty()
            ));
            assert!(visible_topics(connection, recipient)?.is_empty());
            assert!(matches!(
                edit_owned_post(connection, tagged.post.id, recipient, None, Some(&names), None)?,
                PostUpdate::NotFound
            ));
            assert!(matches!(
                edit_owned_post(connection, tagged.post.id, owner, None, Some(&names), None)?,
                PostUpdate::Updated(post) if post.tags.len() == 2 && post.post.title == "Edited"
            ));
            assert!(matches!(
                edit_owned_post(connection, tagged.post.id, owner, Some(("Denied", "Denied")), Some(&[]), None)?,
                PostUpdate::Shared
            ));
            let unchanged = posts::table.find(tagged.post.id).first::<Post>(connection)?;
            let details = PostDetail {
                post: tagged_post(connection, unchanged)?,
                votes: vote_summary(connection, tagged.post.id, owner)?,
                comment_count: 0,
            };
            let json = serde_json::to_value(details).unwrap();
            assert_eq!(json["id"], tagged.post.id);
            assert_eq!(json["tags"].as_array().unwrap().len(), 2);
            assert_eq!(json["score"], 0);
            assert!(json["your_vote"].is_null());
            assert_eq!(json["comment_count"], 0);
            assert_eq!(post_update_response(PostUpdate::Shared).status(), actix_web::http::StatusCode::CONFLICT);
            Ok(())
        });
    }

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
                    image_data: None,
                })
                .returning(Post::as_returning())
                .get_result::<Post>(connection)?;
            assert!(matches!(
                edit_owned_post(
                    connection,
                    post.id,
                    recipient,
                    Some(("Denied", "Denied")),
                    None,
                    None,
                )?,
                PostUpdate::NotFound
            ));
            let updated = match edit_owned_post(
                connection,
                post.id,
                owner,
                Some(("Edited", "Edited body")),
                None,
                None,
            )? {
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
                edit_owned_post(
                    connection,
                    post.id,
                    owner,
                    Some(("Denied", "Denied")),
                    None,
                    None
                )?,
                PostUpdate::Shared
            ));
            let unchanged = posts::table.find(post.id).first::<Post>(connection)?;
            assert_eq!(unchanged.title, updated.post.title);
            assert_eq!(unchanged.body, updated.post.body);
            assert_eq!(unchanged.updated_at, updated.post.updated_at);
            assert!(matches!(
                edit_owned_post(
                    connection,
                    post.id,
                    recipient,
                    Some(("Denied", "Denied")),
                    None,
                    None,
                )?,
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
            tags: None,
            image_data: None,
        })
        .is_ok());
        assert!(validate_post(&PostInput {
            title: "  ".to_owned(),
            body: "A story".to_owned(),
            tags: None,
            image_data: None,
        })
        .is_err());
        assert!(validate_post(&PostInput {
            title: "x".repeat(201),
            body: "A story".to_owned(),
            tags: None,
            image_data: None,
        })
        .is_err());
        assert!(validate_post(&PostInput {
            title: "é".repeat(200),
            body: "A story".to_owned(),
            tags: None,
            image_data: None,
        })
        .is_ok());
        assert!(validate_post(&PostInput {
            title: "A title".to_owned(),
            body: "  ".to_owned(),
            tags: None,
            image_data: None,
        })
        .is_err());
    }

    #[test]
    fn validates_post_images() {
        let valid_image = "data:image/png;base64,aGVsbG8=".to_owned();
        assert!(validate_post(&PostInput {
            title: "A title".to_owned(),
            body: "A story".to_owned(),
            tags: None,
            image_data: Some(valid_image),
        })
        .is_ok());
        for image_data in [
            "data:image/svg+xml;base64,PHN2Zz4=".to_owned(),
            "data:image/png;base64,not base64".to_owned(),
        ] {
            assert!(validate_post(&PostInput {
                title: "A title".to_owned(),
                body: "A story".to_owned(),
                tags: None,
                image_data: Some(image_data),
            })
            .is_err());
        }
    }
}
