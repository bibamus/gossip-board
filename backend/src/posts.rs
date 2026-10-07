use actix_web::{delete, get, post, put, web, HttpRequest, HttpResponse};
use chrono::Utc;
use diesel::prelude::*;
use serde::Deserialize;

use crate::{
    auth::authenticated_user_id,
    db::DbPool,
    models::{NewPost, Post},
    schema::{post_shares, posts},
};

#[derive(Deserialize)]
pub struct PostInput {
    title: String,
    body: String,
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
        let shared_post_ids = post_shares::table
            .filter(post_shares::shared_with_user_id.eq(user_id))
            .select(post_shares::post_id);
        posts::table
            .filter(
                posts::author_id
                    .eq(user_id)
                    .or(posts::id.eq_any(shared_post_ids)),
            )
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
        let shared_post_ids = post_shares::table
            .filter(post_shares::shared_with_user_id.eq(user_id))
            .select(post_shares::post_id);
        posts::table
            .filter(posts::id.eq(post_id))
            .filter(
                posts::author_id
                    .eq(user_id)
                    .or(posts::id.eq_any(shared_post_ids)),
            )
            .select(Post::as_select())
            .first::<Post>(&mut connection)
            .optional()
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
    let updated_at = Utc::now();

    let result = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        diesel::update(posts::table.filter(posts::id.eq(post_id).and(posts::author_id.eq(user_id))))
            .set((
                posts::title.eq(title),
                posts::body.eq(body),
                posts::updated_at.eq(updated_at),
            ))
            .returning(Post::as_returning())
            .get_result::<Post>(&mut connection)
            .optional()
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
    use super::{validate_post, PostInput};

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
