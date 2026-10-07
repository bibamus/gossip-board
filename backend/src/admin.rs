use actix_web::{delete, get, web, HttpRequest, HttpResponse};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use serde::Serialize;

use crate::{
    auth::{authenticated_user_id, is_admin_email},
    db::DbPool,
    schema::{posts, users},
};

#[derive(Serialize)]
struct AdminUser {
    id: i64,
    email: String,
    username: String,
    created_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct AdminOverview {
    users: Vec<AdminUser>,
    post_count: i64,
}

enum DeleteUserResult {
    Deleted,
    Forbidden,
    SelfDeletion,
    NotFound,
}

#[get("/api/admin/overview")]
pub async fn overview(
    pool: web::Data<DbPool>,
    request: HttpRequest,
) -> actix_web::Result<HttpResponse> {
    let Some(user_id) = authenticated_user_id(pool.clone(), &request).await? else {
        return Ok(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Sign in to view the admin overview."
        })));
    };

    let result = web::block(move || -> Result<Option<AdminOverview>, String> {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        let email = users::table
            .find(user_id)
            .select(users::email)
            .first::<String>(&mut connection)
            .map_err(|error| error.to_string())?;

        if !is_admin_email(&email) {
            return Ok(None);
        }

        let user_rows = users::table
            .select((users::id, users::email, users::username, users::created_at))
            .order(users::created_at.desc())
            .load::<(i64, String, String, DateTime<Utc>)>(&mut connection)
            .map_err(|error| error.to_string())?;
        let post_count = posts::table
            .count()
            .get_result::<i64>(&mut connection)
            .map_err(|error| error.to_string())?;

        Ok(Some(AdminOverview {
            users: user_rows
                .into_iter()
                .map(|(id, email, username, created_at)| AdminUser {
                    id,
                    email,
                    username,
                    created_at,
                })
                .collect(),
            post_count,
        }))
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;

    match result {
        Some(overview) => Ok(HttpResponse::Ok().json(overview)),
        None => Ok(HttpResponse::Forbidden().json(serde_json::json!({
            "error": "You do not have permission to view the admin overview."
        }))),
    }
}

#[delete("/api/admin/users/{id}")]
pub async fn delete_user(
    pool: web::Data<DbPool>,
    request: HttpRequest,
    target_id: web::Path<i64>,
) -> actix_web::Result<HttpResponse> {
    let Some(admin_id) = authenticated_user_id(pool.clone(), &request).await? else {
        return Ok(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Sign in to manage users."
        })));
    };
    let target_id = target_id.into_inner();

    let result = web::block(move || -> Result<DeleteUserResult, String> {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        let admin_email = users::table
            .find(admin_id)
            .select(users::email)
            .first::<String>(&mut connection)
            .optional()
            .map_err(|error| error.to_string())?;
        let Some(admin_email) = admin_email else {
            return Ok(DeleteUserResult::Forbidden);
        };
        if !is_admin_email(&admin_email) {
            return Ok(DeleteUserResult::Forbidden);
        }
        if admin_id == target_id {
            return Ok(DeleteUserResult::SelfDeletion);
        }

        let deleted = diesel::delete(users::table.find(target_id))
            .execute(&mut connection)
            .map_err(|error| error.to_string())?;
        if deleted == 0 {
            Ok(DeleteUserResult::NotFound)
        } else {
            Ok(DeleteUserResult::Deleted)
        }
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;

    match result {
        DeleteUserResult::Deleted => Ok(HttpResponse::NoContent().finish()),
        DeleteUserResult::Forbidden => Ok(HttpResponse::Forbidden().json(serde_json::json!({
            "error": "You do not have permission to manage users."
        }))),
        DeleteUserResult::SelfDeletion => Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "You cannot delete your own account from the admin overview."
        }))),
        DeleteUserResult::NotFound => Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "That user no longer exists."
        }))),
    }
}
