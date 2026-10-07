mod auth;
mod db;
mod models;
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

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .service(health)
            .service(auth::request_link)
            .service(auth::verify_link)
            .service(auth::current_user)
            .service(auth::logout)
    })
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}
