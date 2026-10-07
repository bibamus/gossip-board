mod auth;
mod db;
mod interactions;
mod models;
mod posts;
mod schema;

use actix_web::{get, web, App, HttpResponse, HttpServer, Responder};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

use crate::db::create_pool;

const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

#[get("/health")]
async fn health() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "ok",
        "service": "gossip-board-backend"
    }))
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init();

    let database_url = std::env::var("DATABASE_URL").map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("DATABASE_URL must be set: {error}"),
        )
    })?;
    let pool = create_pool(&database_url).map_err(|error| {
        std::io::Error::other(format!("failed to create database pool: {error}"))
    })?;

    let mut connection = pool.get().map_err(|error| {
        std::io::Error::other(format!("failed to connect to database: {error}"))
    })?;
    connection
        .run_pending_migrations(MIGRATIONS)
        .map_err(|error| {
            std::io::Error::other(format!("failed to run database migrations: {error}"))
        })?;
    drop(connection);

    let bind_address =
        std::env::var("BIND_ADDRESS").unwrap_or_else(|_| "127.0.0.1:8080".to_owned());

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .service(health)
            .service(auth::request_link)
            .service(auth::verify_link)
            .service(auth::current_user)
            .service(auth::logout)
            .service(posts::list_posts)
            .service(posts::get_post)
            .service(posts::create_post)
            .service(posts::update_post)
            .service(posts::delete_post)
            .service(posts::list_shares)
            .service(posts::search_users)
            .service(posts::share_post)
            .service(interactions::get_comments)
            .service(interactions::create_comment)
            .service(interactions::create_vote)
            .service(interactions::update_vote)
            .service(interactions::delete_vote)
    })
    .bind(bind_address)?
    .run()
    .await
}
