use std::time::Duration;

use actix_web::{
    cookie::{Cookie, SameSite},
    delete, get, post, put, web, HttpRequest, HttpResponse, Responder,
};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use lettre::{
    message::Mailbox, transport::smtp::authentication::Credentials, Message, SmtpTransport,
    Transport,
};
use rand::{rngs::SysRng, TryRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    db::DbPool,
    models::{NewUser, User},
    schema::{magic_link_tokens, sessions, users},
};

const MAGIC_LINK_LIFETIME: Duration = Duration::from_secs(15 * 60);
const SESSION_LIFETIME: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const SESSION_COOKIE: &str = "gossip_session";

#[derive(Debug, PartialEq)]
enum SmtpTls {
    None,
    Starttls,
    Implicit,
}

impl SmtpTls {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "none" => Ok(Self::None),
            "starttls" => Ok(Self::Starttls),
            "implicit" => Ok(Self::Implicit),
            _ => Err("SMTP_TLS must be none, starttls, or implicit".to_owned()),
        }
    }
}

#[derive(Deserialize)]
pub struct RequestLink {
    email: String,
}

#[derive(Deserialize)]
pub struct VerifyLink {
    token: String,
}

#[derive(Deserialize)]
pub struct UpdateUsername {
    username: String,
}

#[derive(Serialize)]
struct AuthUser {
    id: i64,
    email: String,
    username: String,
}

impl From<User> for AuthUser {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            email: user.email,
            username: user.username,
        }
    }
}

#[derive(Serialize)]
struct AuthResponse {
    user: AuthUser,
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    SysRng
        .try_fill_bytes(&mut bytes)
        .expect("system random number source is unavailable");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn token_hash(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn normalize_username(value: &str) -> Result<String, &'static str> {
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

pub(crate) fn valid_email(email: &str) -> bool {
    if email.len() > 320 || email.trim() != email || !email.is_ascii() {
        return false;
    }
    let mut parts = email.split('@');
    let local = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    parts.next().is_none()
        && !local.is_empty()
        && local.len() <= 64
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(
                    character,
                    '.' | '!'
                        | '#'
                        | '$'
                        | '%'
                        | '&'
                        | '\''
                        | '*'
                        | '+'
                        | '-'
                        | '/'
                        | '='
                        | '?'
                        | '^'
                        | '_'
                        | '`'
                        | '{'
                        | '|'
                        | '}'
                        | '~'
                )
        })
        && !domain.is_empty()
        && domain.contains('.')
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        })
        && !email.chars().any(char::is_whitespace)
}

fn cookie_secure() -> bool {
    std::env::var("COOKIE_SECURE")
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or_else(|_| {
            std::env::var("APP_ENV")
                .map(|value| value.eq_ignore_ascii_case("production"))
                .unwrap_or(false)
        })
}

fn send_magic_link(email: &str, link: &str) -> Result<(), String> {
    let Ok(host) = std::env::var("SMTP_HOST") else {
        if std::env::var("APP_ENV")
            .map(|value| value.eq_ignore_ascii_case("production"))
            .unwrap_or(false)
        {
            return Err("SMTP_HOST must be configured in production".to_owned());
        }
        println!("Magic link for {email}: {link}");
        return Ok(());
    };

    let from: Mailbox = std::env::var("SMTP_FROM")
        .map_err(|_| "SMTP_FROM must be configured when SMTP_HOST is set".to_owned())?
        .parse()
        .map_err(|error| format!("invalid SMTP_FROM address: {error}"))?;
    let to: Mailbox = email
        .parse()
        .map_err(|error| format!("invalid recipient address: {error}"))?;
    let message = Message::builder()
        .from(from)
        .to(to)
        .subject("Your Gossip Board sign-in link")
        .body(format!(
            "Use this one-time link to sign in to Gossip Board:\n\n{link}\n\nThis link expires in 15 minutes and can only be used once."
        ))
        .map_err(|error| format!("failed to build sign-in email: {error}"))?;
    let port = std::env::var("SMTP_PORT")
        .unwrap_or_else(|_| "587".to_owned())
        .parse::<u16>()
        .map_err(|error| format!("invalid SMTP_PORT: {error}"))?;

    let tls = SmtpTls::parse(&std::env::var("SMTP_TLS").unwrap_or_else(|_| "implicit".to_owned()))?;
    let mut builder = match tls {
        SmtpTls::None => SmtpTransport::builder_dangerous(&host),
        SmtpTls::Starttls => SmtpTransport::starttls_relay(&host)
            .map_err(|error| format!("failed to configure SMTP transport: {error}"))?,
        SmtpTls::Implicit => SmtpTransport::relay(&host)
            .map_err(|error| format!("failed to configure SMTP transport: {error}"))?,
    }
    .port(port);
    match (
        std::env::var("SMTP_USERNAME"),
        std::env::var("SMTP_PASSWORD"),
    ) {
        (Ok(username), Ok(password)) => {
            builder = builder.credentials(Credentials::new(username, password));
        }
        (Err(_), Err(_)) => {}
        _ => return Err("SMTP_USERNAME and SMTP_PASSWORD must be set together".to_owned()),
    }

    builder
        .build()
        .send(&message)
        .map_err(|error| format!("failed to send sign-in email: {error}"))?;
    Ok(())
}

#[post("/api/auth/request-link")]
pub async fn request_link(
    pool: web::Data<DbPool>,
    body: web::Json<RequestLink>,
) -> actix_web::Result<impl Responder> {
    let email = body.email.trim().to_ascii_lowercase();
    if !valid_email(&email) {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Enter a valid email address."
        })));
    }

    let token = random_token();
    let hash = token_hash(&token);
    let now = Utc::now();
    let expires_at = now
        + chrono::Duration::from_std(MAGIC_LINK_LIFETIME)
            .expect("magic-link lifetime is within chrono's supported range");
    let local_part = email.split('@').next().unwrap_or("user");
    let username = format!(
        "{}-{}",
        local_part
            .chars()
            .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
            .take(38)
            .collect::<String>(),
        &token[..8]
    );
    let email_for_db = email.clone();
    let username_for_db = username;
    let hash_for_db = hash.clone();

    web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        connection
            .transaction::<(), diesel::result::Error, _>(|connection| {
                diesel::delete(
                    magic_link_tokens::table.filter(magic_link_tokens::expires_at.le(now)),
                )
                .execute(connection)?;
                diesel::delete(
                    magic_link_tokens::table.filter(magic_link_tokens::used_at.is_not_null()),
                )
                .execute(connection)?;
                diesel::delete(sessions::table.filter(sessions::expires_at.le(now)))
                    .execute(connection)?;
                diesel::insert_into(users::table)
                    .values(NewUser {
                        email: &email_for_db,
                        username: &username_for_db,
                    })
                    .on_conflict_do_nothing()
                    .execute(connection)?;
                let user_id = users::table
                    .filter(
                        diesel::dsl::sql::<diesel::sql_types::Bool>("LOWER(email) = ")
                            .bind::<diesel::sql_types::Text, _>(&email_for_db),
                    )
                    .select(users::id)
                    .first::<i64>(connection)?;
                diesel::delete(
                    magic_link_tokens::table
                        .filter(magic_link_tokens::user_id.eq(user_id))
                        .filter(magic_link_tokens::used_at.is_null()),
                )
                .execute(connection)?;
                diesel::insert_into(magic_link_tokens::table)
                    .values((
                        magic_link_tokens::user_id.eq(user_id),
                        magic_link_tokens::token_hash.eq(hash_for_db),
                        magic_link_tokens::expires_at.eq(expires_at),
                    ))
                    .execute(connection)?;
                Ok(())
            })
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;

    let app_base_url =
        std::env::var("APP_BASE_URL").unwrap_or_else(|_| "http://localhost:5173".to_owned());
    let link = format!("{}/?token={token}", app_base_url.trim_end_matches('/'));
    let email_for_send = email.clone();
    web::block(move || send_magic_link(&email_for_send, &link))
        .await
        .map_err(actix_web::error::ErrorInternalServerError)?
        .map_err(|error| {
            log::error!("Failed to deliver magic link: {error}");
            actix_web::error::ErrorInternalServerError("unable to deliver sign-in link")
        })?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": "If this email can receive sign-in links, one is on its way."
    })))
}

#[post("/api/auth/verify")]
pub async fn verify_link(
    pool: web::Data<DbPool>,
    body: web::Json<VerifyLink>,
) -> actix_web::Result<impl Responder> {
    if body.token.len() != 64 || !body.token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "This sign-in link is invalid or has expired."
        })));
    }

    let token_hash_value = token_hash(&body.token);
    let session_token = random_token();
    let session_hash_value = token_hash(&session_token);
    let now = Utc::now();
    let session_expires_at = now
        + chrono::Duration::from_std(SESSION_LIFETIME)
            .expect("session lifetime is within chrono's supported range");
    let hash_for_db = token_hash_value.clone();

    let user = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        let result = connection.transaction::<User, diesel::result::Error, _>(|connection| {
            let (token_id, user_id) = magic_link_tokens::table
                .filter(magic_link_tokens::token_hash.eq(hash_for_db))
                .filter(magic_link_tokens::used_at.is_null())
                .filter(magic_link_tokens::expires_at.gt(now))
                .select((magic_link_tokens::id, magic_link_tokens::user_id))
                .for_update()
                .first::<(i64, i64)>(connection)?;
            diesel::update(magic_link_tokens::table.find(token_id))
                .set(magic_link_tokens::used_at.eq(Some(now)))
                .execute(connection)?;
            diesel::update(users::table.find(user_id))
                .set(users::last_login_at.eq(Some(now)))
                .execute(connection)?;
            diesel::insert_into(sessions::table)
                .values((
                    sessions::user_id.eq(user_id),
                    sessions::session_hash.eq(session_hash_value),
                    sessions::expires_at.eq(session_expires_at),
                ))
                .execute(connection)?;
            users::table.find(user_id).first::<User>(connection)
        });
        Ok::<_, String>(result)
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?;

    let user = match user {
        Ok(Ok(user)) => user,
        Ok(Err(diesel::result::Error::NotFound)) => {
            return Ok(HttpResponse::Unauthorized().json(serde_json::json!({
                "error": "This sign-in link is invalid or has expired."
            })));
        }
        Ok(Err(error)) => return Err(actix_web::error::ErrorInternalServerError(error)),
        Err(error) => return Err(actix_web::error::ErrorInternalServerError(error)),
    };

    let mut cookie = Cookie::build(SESSION_COOKIE, session_token)
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(actix_web::cookie::time::Duration::seconds(
            SESSION_LIFETIME.as_secs() as i64,
        ));
    if cookie_secure() {
        cookie = cookie.secure(true);
    }

    Ok(HttpResponse::Ok()
        .cookie(cookie.finish())
        .json(AuthResponse { user: user.into() }))
}

#[get("/api/auth/me")]
pub async fn current_user(pool: web::Data<DbPool>, request: HttpRequest) -> HttpResponse {
    let Some(session_token) = request
        .cookie(SESSION_COOKIE)
        .map(|cookie| cookie.value().to_owned())
    else {
        return HttpResponse::Unauthorized().finish();
    };
    let hash = token_hash(&session_token);
    let now: DateTime<Utc> = Utc::now();

    let user = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        let result = sessions::table
            .inner_join(users::table)
            .filter(sessions::session_hash.eq(hash))
            .filter(sessions::expires_at.gt(now))
            .select(User::as_select())
            .first::<User>(&mut connection);
        Ok::<_, String>(result)
    })
    .await;

    match user {
        Ok(Ok(Ok(user))) => HttpResponse::Ok().json(AuthResponse { user: user.into() }),
        Ok(Ok(Err(diesel::result::Error::NotFound))) => HttpResponse::Unauthorized().finish(),
        Ok(Ok(Err(error))) => {
            log::error!("Failed to load authenticated user: {error}");
            HttpResponse::InternalServerError().finish()
        }
        Ok(Err(error)) => {
            log::error!("Failed to get a database connection: {error}");
            HttpResponse::InternalServerError().finish()
        }
        Err(error) => {
            log::error!("Failed to load authenticated user: {error}");
            HttpResponse::InternalServerError().finish()
        }
    }
}

#[put("/api/auth/me")]
pub async fn update_username(
    pool: web::Data<DbPool>,
    request: HttpRequest,
    input: web::Json<UpdateUsername>,
) -> actix_web::Result<HttpResponse> {
    let Some(user_id) = authenticated_user_id(pool.clone(), &request).await? else {
        return Ok(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Sign in to change your username."
        })));
    };
    let username = match normalize_username(&input.username) {
        Ok(username) => username,
        Err(error) => {
            return Ok(HttpResponse::BadRequest().json(serde_json::json!({ "error": error })));
        }
    };

    let result = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        Ok::<_, String>(
            diesel::update(users::table.find(user_id))
                .set(users::username.eq(username))
                .get_result::<User>(&mut connection),
        )
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?;

    match result {
        Err(error) => Err(actix_web::error::ErrorInternalServerError(error)),
        Ok(Err(diesel::result::Error::NotFound)) => Ok(HttpResponse::Unauthorized().finish()),
        Ok(Err(diesel::result::Error::DatabaseError(
            diesel::result::DatabaseErrorKind::UniqueViolation,
            _,
        ))) => Ok(HttpResponse::Conflict().json(serde_json::json!({
            "error": "That username is already in use."
        }))),
        Ok(Err(error)) => Err(actix_web::error::ErrorInternalServerError(error)),
        Ok(Ok(user)) => Ok(HttpResponse::Ok().json(AuthResponse { user: user.into() })),
    }
}

#[delete("/api/auth/me")]
pub async fn delete_account(
    pool: web::Data<DbPool>,
    request: HttpRequest,
) -> actix_web::Result<HttpResponse> {
    let Some(user_id) = authenticated_user_id(pool.clone(), &request).await? else {
        return Ok(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Sign in to delete your account."
        })));
    };

    let result = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        diesel::delete(users::table.find(user_id))
            .execute(&mut connection)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?;
    result.map_err(actix_web::error::ErrorInternalServerError)?;

    let mut expired_cookie = Cookie::build(SESSION_COOKIE, "")
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(actix_web::cookie::time::Duration::seconds(0));
    if cookie_secure() {
        expired_cookie = expired_cookie.secure(true);
    }
    Ok(HttpResponse::NoContent()
        .cookie(expired_cookie.finish())
        .finish())
}

pub async fn authenticated_user_id(
    pool: web::Data<DbPool>,
    request: &HttpRequest,
) -> actix_web::Result<Option<i64>> {
    let Some(session_token) = request
        .cookie(SESSION_COOKIE)
        .map(|cookie| cookie.value().to_owned())
    else {
        return Ok(None);
    };
    let hash = token_hash(&session_token);
    let now = Utc::now();

    let user_id = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        sessions::table
            .filter(sessions::session_hash.eq(hash))
            .filter(sessions::expires_at.gt(now))
            .select(sessions::user_id)
            .first::<i64>(&mut connection)
            .optional()
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;

    Ok(user_id)
}

#[post("/api/auth/logout")]
pub async fn logout(pool: web::Data<DbPool>, request: HttpRequest) -> HttpResponse {
    if let Some(cookie) = request.cookie(SESSION_COOKIE) {
        let hash = token_hash(cookie.value());
        let result = web::block(move || {
            let mut connection = pool.get().map_err(|error| error.to_string())?;
            diesel::delete(sessions::table.filter(sessions::session_hash.eq(hash)))
                .execute(&mut connection)
                .map_err(|error| error.to_string())
        })
        .await;

        match result {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                log::error!("Failed to revoke session: {error}");
                return HttpResponse::InternalServerError().finish();
            }
            Err(error) => {
                log::error!("Failed to revoke session: {error}");
                return HttpResponse::InternalServerError().finish();
            }
        }
    }

    let mut expired_cookie = Cookie::build(SESSION_COOKIE, "")
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(actix_web::cookie::time::Duration::seconds(0));
    if cookie_secure() {
        expired_cookie = expired_cookie.secure(true);
    }
    HttpResponse::Ok().cookie(expired_cookie.finish()).finish()
}

#[cfg(test)]
mod tests {
    use super::{normalize_username, random_token, token_hash, valid_email, SmtpTls};

    #[test]
    fn smtp_tls_modes_are_explicit_and_invalid_values_are_rejected() {
        assert_eq!(SmtpTls::parse("none"), Ok(SmtpTls::None));
        assert_eq!(SmtpTls::parse("starttls"), Ok(SmtpTls::Starttls));
        assert_eq!(SmtpTls::parse("implicit"), Ok(SmtpTls::Implicit));
        assert!(SmtpTls::parse("").is_err());
        assert!(SmtpTls::parse("true").is_err());
    }

    #[test]
    fn accepts_and_normalizes_common_email_addresses() {
        assert!(valid_email("person@example.com"));
        assert!(valid_email("person+tag@sub.example.com"));
        assert!(!valid_email("not-an-email"));
        assert!(!valid_email("person@localhost"));
        assert!(!valid_email("person @example.com"));
        assert!(!valid_email("person@bad..example.com"));
        assert!(!valid_email("person@-example.com"));
    }

    #[test]
    fn tokens_are_random_and_stored_as_one_way_hashes() {
        let first = random_token();
        let second = random_token();
        assert_eq!(first.len(), 64);
        assert_ne!(first, second);
        assert_eq!(token_hash(&first).len(), 64);
        assert_ne!(token_hash(&first), first);
    }

    #[test]
    fn usernames_are_normalized_and_validated() {
        assert_eq!(
            normalize_username("  @Some.Name_1  "),
            Ok("some.name_1".to_owned())
        );
        assert!(normalize_username("").is_err());
        assert!(normalize_username("invalid name").is_err());
        assert!(normalize_username(&"a".repeat(51)).is_err());
    }
}
