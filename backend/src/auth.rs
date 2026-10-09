use std::time::Duration;

use actix_web::{
    cookie::{Cookie, SameSite},
    delete, get, post, put, web, HttpRequest, HttpResponse,
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
    rate_limit::{client_ip, RateLimiter},
    schema::{magic_link_tokens, pending_signups, sessions, users},
};

const MAGIC_LINK_LIFETIME: Duration = Duration::from_secs(15 * 60);
const SESSION_LIFETIME: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const PENDING_SIGNUP_LIFETIME: Duration = Duration::from_secs(60 * 60);
const SESSION_COOKIE: &str = "gossip_session";
const SIGNUP_COOKIE: &str = "gossip_signup";
/// Unexpired links per email. Tokens are only purged after expiry, so this is a
/// limit per `MAGIC_LINK_LIFETIME` window.
const LINKS_PER_EMAIL: usize = 3;
pub(crate) const LINKS_PER_IP: usize = 10;
pub(crate) const LINK_RATE_WINDOW: Duration = MAGIC_LINK_LIFETIME;

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
pub struct UsernameInput {
    username: String,
}

#[derive(Serialize)]
struct AuthUser {
    id: i64,
    email: String,
    username: String,
    is_admin: bool,
}

impl From<User> for AuthUser {
    fn from(user: User) -> Self {
        let is_admin = is_admin_email(&user.email);
        Self {
            id: user.id,
            email: user.email,
            username: user.username,
            is_admin,
        }
    }
}

#[derive(Serialize)]
struct PendingSignup {
    email: String,
}

/// Returned instead of `AuthResponse` when a verified email has no account yet.
#[derive(Serialize)]
struct SignupResponse {
    signup: PendingSignup,
}

pub(crate) fn is_admin_email(email: &str) -> bool {
    std::env::var("ADMIN_EMAILS")
        .unwrap_or_default()
        .split(',')
        .any(|configured| {
            !configured.trim().is_empty() && configured.trim().eq_ignore_ascii_case(email)
        })
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

fn auth_cookie(name: &'static str, value: String, lifetime: Duration) -> Cookie<'static> {
    let mut cookie = Cookie::build(name, value)
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(actix_web::cookie::time::Duration::seconds(
            lifetime.as_secs() as i64,
        ));
    if cookie_secure() {
        cookie = cookie.secure(true);
    }
    cookie.finish()
}

fn expired_cookie(name: &'static str) -> Cookie<'static> {
    auth_cookie(name, String::new(), Duration::ZERO)
}

fn cookie_hash(request: &HttpRequest, name: &str) -> Option<String> {
    request
        .cookie(name)
        .map(|cookie| token_hash(cookie.value()))
}

fn expires_after(now: DateTime<Utc>, lifetime: Duration) -> DateTime<Utc> {
    now + chrono::Duration::from_std(lifetime).expect("lifetime is within chrono's supported range")
}

fn error_response(mut builder: actix_web::HttpResponseBuilder, message: &str) -> HttpResponse {
    builder.json(serde_json::json!({ "error": message }))
}

fn too_many_requests(retry_after: Duration, message: &str) -> HttpResponse {
    let mut builder = HttpResponse::TooManyRequests();
    builder.insert_header((
        actix_web::http::header::RETRY_AFTER,
        retry_after.as_secs().max(1).to_string(),
    ));
    error_response(builder, message)
}

/// Serializes concurrent work on the same email until the transaction ends.
fn lock_email(connection: &mut PgConnection, email: &str) -> QueryResult<()> {
    diesel::sql_query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind::<diesel::sql_types::Text, _>(email)
        .execute(connection)?;
    Ok(())
}

fn find_user_by_email(connection: &mut PgConnection, email: &str) -> QueryResult<Option<User>> {
    users::table
        .filter(
            diesel::dsl::sql::<diesel::sql_types::Bool>("LOWER(email) = ")
                .bind::<diesel::sql_types::Text, _>(email),
        )
        .select(User::as_select())
        .first::<User>(connection)
        .optional()
}

fn start_session(
    connection: &mut PgConnection,
    user_id: i64,
    session_hash: &str,
    now: DateTime<Utc>,
) -> QueryResult<User> {
    diesel::insert_into(sessions::table)
        .values((
            sessions::user_id.eq(user_id),
            sessions::session_hash.eq(session_hash),
            sessions::expires_at.eq(expires_after(now, SESSION_LIFETIME)),
        ))
        .execute(connection)?;
    diesel::update(users::table.find(user_id))
        .set(users::last_login_at.eq(Some(now)))
        .returning(User::as_returning())
        .get_result(connection)
}

fn session_user(
    connection: &mut PgConnection,
    session_hash: &str,
    now: DateTime<Utc>,
) -> QueryResult<Option<User>> {
    sessions::table
        .inner_join(users::table)
        .filter(sessions::session_hash.eq(session_hash))
        .filter(sessions::expires_at.gt(now))
        .select(User::as_select())
        .first::<User>(connection)
        .optional()
}

fn pending_signup_email(
    connection: &mut PgConnection,
    signup_hash: &str,
    now: DateTime<Utc>,
) -> QueryResult<Option<String>> {
    pending_signups::table
        .filter(pending_signups::signup_hash.eq(signup_hash))
        .filter(pending_signups::expires_at.gt(now))
        .select(pending_signups::email)
        .first::<String>(connection)
        .optional()
}

enum LinkRequest {
    Issued,
    Limited(Duration),
}

fn issue_magic_link(
    connection: &mut PgConnection,
    email: &str,
    token_hash: &str,
    now: DateTime<Utc>,
) -> QueryResult<LinkRequest> {
    connection.transaction(|connection| {
        diesel::delete(magic_link_tokens::table.filter(magic_link_tokens::expires_at.le(now)))
            .execute(connection)?;
        diesel::delete(sessions::table.filter(sessions::expires_at.le(now))).execute(connection)?;
        diesel::delete(pending_signups::table.filter(pending_signups::expires_at.le(now)))
            .execute(connection)?;

        lock_email(connection, email)?;
        let active = magic_link_tokens::table
            .filter(magic_link_tokens::email.eq(email))
            .filter(magic_link_tokens::expires_at.gt(now))
            .order(magic_link_tokens::expires_at.asc())
            .select(magic_link_tokens::expires_at)
            .load::<DateTime<Utc>>(connection)?;
        if active.len() >= LINKS_PER_EMAIL {
            let retry_after = (active[0] - now).to_std().unwrap_or_default();
            return Ok(LinkRequest::Limited(retry_after));
        }

        diesel::insert_into(magic_link_tokens::table)
            .values((
                magic_link_tokens::email.eq(email),
                magic_link_tokens::token_hash.eq(token_hash),
                magic_link_tokens::expires_at.eq(expires_after(now, MAGIC_LINK_LIFETIME)),
            ))
            .execute(connection)?;
        Ok(LinkRequest::Issued)
    })
}

#[post("/api/auth/request-link")]
pub async fn request_link(
    pool: web::Data<DbPool>,
    limiter: web::Data<RateLimiter>,
    request: HttpRequest,
    body: web::Json<RequestLink>,
) -> actix_web::Result<HttpResponse> {
    if let Some(ip) = client_ip(&request) {
        if let Err(retry_after) = limiter.check(&ip) {
            return Ok(too_many_requests(
                retry_after,
                "Too many sign-in links were requested. Try again later.",
            ));
        }
    }

    let email = body.email.trim().to_ascii_lowercase();
    if !valid_email(&email) {
        return Ok(error_response(
            HttpResponse::BadRequest(),
            "Enter a valid email address.",
        ));
    }

    let token = random_token();
    let hash = token_hash(&token);
    let now = Utc::now();
    let email_for_db = email.clone();

    let outcome = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        issue_magic_link(&mut connection, &email_for_db, &hash, now)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;

    if let LinkRequest::Limited(retry_after) = outcome {
        return Ok(too_many_requests(
            retry_after,
            "Too many sign-in links were requested for this email. Try again in a few minutes.",
        ));
    }

    let app_base_url =
        std::env::var("APP_BASE_URL").unwrap_or_else(|_| "http://localhost:5173".to_owned());
    let link = format!("{}/?token={token}", app_base_url.trim_end_matches('/'));
    web::block(move || send_magic_link(&email, &link))
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

enum VerifyOutcome {
    SignedIn(User),
    Signup(String),
}

#[post("/api/auth/verify")]
pub async fn verify_link(
    pool: web::Data<DbPool>,
    request: HttpRequest,
    body: web::Json<VerifyLink>,
) -> actix_web::Result<HttpResponse> {
    const INVALID_LINK: &str = "This sign-in link is invalid or has expired.";
    if body.token.len() != 64 || !body.token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(error_response(HttpResponse::BadRequest(), INVALID_LINK));
    }

    let link_hash = token_hash(&body.token);
    // Becomes either the session token or the pending sign-up token.
    let new_token = random_token();
    let new_hash = token_hash(&new_token);
    let previous_session = cookie_hash(&request, SESSION_COOKIE);
    let previous_signup = cookie_hash(&request, SIGNUP_COOKIE);
    let now = Utc::now();

    let outcome = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        Ok::<_, String>(
            connection.transaction::<_, diesel::result::Error, _>(|connection| {
                let (token_id, email) = magic_link_tokens::table
                    .filter(magic_link_tokens::token_hash.eq(&link_hash))
                    .filter(magic_link_tokens::used_at.is_null())
                    .filter(magic_link_tokens::expires_at.gt(now))
                    .select((magic_link_tokens::id, magic_link_tokens::email))
                    .for_update()
                    .first::<(i64, String)>(connection)?;
                diesel::update(magic_link_tokens::table.find(token_id))
                    .set(magic_link_tokens::used_at.eq(Some(now)))
                    .execute(connection)?;

                // A sign-in link replaces whatever session or sign-up this browser had.
                if let Some(hash) = &previous_session {
                    diesel::delete(sessions::table.filter(sessions::session_hash.eq(hash)))
                        .execute(connection)?;
                }
                if let Some(hash) = &previous_signup {
                    diesel::delete(
                        pending_signups::table.filter(pending_signups::signup_hash.eq(hash)),
                    )
                    .execute(connection)?;
                }

                match find_user_by_email(connection, &email)? {
                    Some(user) => Ok(VerifyOutcome::SignedIn(start_session(
                        connection, user.id, &new_hash, now,
                    )?)),
                    None => {
                        diesel::insert_into(pending_signups::table)
                            .values((
                                pending_signups::email.eq(&email),
                                pending_signups::signup_hash.eq(&new_hash),
                                pending_signups::expires_at
                                    .eq(expires_after(now, PENDING_SIGNUP_LIFETIME)),
                            ))
                            .execute(connection)?;
                        Ok(VerifyOutcome::Signup(email))
                    }
                }
            }),
        )
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;

    match outcome {
        Ok(VerifyOutcome::SignedIn(user)) => Ok(HttpResponse::Ok()
            .cookie(auth_cookie(SESSION_COOKIE, new_token, SESSION_LIFETIME))
            .cookie(expired_cookie(SIGNUP_COOKIE))
            .json(AuthResponse { user: user.into() })),
        Ok(VerifyOutcome::Signup(email)) => Ok(HttpResponse::Ok()
            .cookie(auth_cookie(
                SIGNUP_COOKIE,
                new_token,
                PENDING_SIGNUP_LIFETIME,
            ))
            .cookie(expired_cookie(SESSION_COOKIE))
            .json(SignupResponse {
                signup: PendingSignup { email },
            })),
        Err(diesel::result::Error::NotFound) => {
            Ok(error_response(HttpResponse::Unauthorized(), INVALID_LINK))
        }
        Err(error) => Err(actix_web::error::ErrorInternalServerError(error)),
    }
}

#[get("/api/auth/me")]
pub async fn current_user(
    pool: web::Data<DbPool>,
    request: HttpRequest,
) -> actix_web::Result<HttpResponse> {
    let session = cookie_hash(&request, SESSION_COOKIE);
    let signup = cookie_hash(&request, SIGNUP_COOKIE);
    if session.is_none() && signup.is_none() {
        return Ok(HttpResponse::Unauthorized().finish());
    }
    let now = Utc::now();

    let (user, signup_email) = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        if let Some(hash) = session {
            let user =
                session_user(&mut connection, &hash, now).map_err(|error| error.to_string())?;
            if user.is_some() {
                return Ok((user, None));
            }
        }
        let email = match signup {
            Some(hash) => pending_signup_email(&mut connection, &hash, now)
                .map_err(|error| error.to_string())?,
            None => None,
        };
        Ok::<_, String>((None, email))
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;

    Ok(match (user, signup_email) {
        (Some(user), _) => HttpResponse::Ok().json(AuthResponse { user: user.into() }),
        (None, Some(email)) => HttpResponse::Ok().json(SignupResponse {
            signup: PendingSignup { email },
        }),
        (None, None) => HttpResponse::Unauthorized().finish(),
    })
}

enum SignupError {
    Expired,
    Database(diesel::result::Error),
}

impl From<diesel::result::Error> for SignupError {
    fn from(error: diesel::result::Error) -> Self {
        Self::Database(error)
    }
}

fn create_account(
    connection: &mut PgConnection,
    signup_hash: &str,
    username: &str,
    session_hash: &str,
    now: DateTime<Utc>,
) -> Result<User, SignupError> {
    connection.transaction(|connection| {
        let email =
            pending_signup_email(connection, signup_hash, now)?.ok_or(SignupError::Expired)?;
        lock_email(connection, &email)?;
        diesel::delete(pending_signups::table.filter(pending_signups::email.eq(&email)))
            .execute(connection)?;
        // Another browser may have completed a sign-up for this email first.
        let user = match find_user_by_email(connection, &email)? {
            Some(user) => user,
            None => diesel::insert_into(users::table)
                .values(NewUser {
                    email: &email,
                    username,
                })
                .returning(User::as_returning())
                .get_result(connection)?,
        };
        Ok(start_session(connection, user.id, session_hash, now)?)
    })
}

#[post("/api/auth/signup")]
pub async fn complete_signup(
    pool: web::Data<DbPool>,
    request: HttpRequest,
    input: web::Json<UsernameInput>,
) -> actix_web::Result<HttpResponse> {
    const EXPIRED: &str = "Your sign-up has expired. Request a new sign-in link.";
    let Some(signup_hash) = cookie_hash(&request, SIGNUP_COOKIE) else {
        return Ok(error_response(HttpResponse::Unauthorized(), EXPIRED));
    };
    let username = match normalize_username(&input.username) {
        Ok(username) => username,
        Err(error) => return Ok(error_response(HttpResponse::BadRequest(), error)),
    };
    let session_token = random_token();
    let session_hash = token_hash(&session_token);
    let now = Utc::now();

    let result = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        Ok::<_, String>(create_account(
            &mut connection,
            &signup_hash,
            &username,
            &session_hash,
            now,
        ))
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;

    match result {
        Ok(user) => Ok(HttpResponse::Created()
            .cookie(auth_cookie(SESSION_COOKIE, session_token, SESSION_LIFETIME))
            .cookie(expired_cookie(SIGNUP_COOKIE))
            .json(AuthResponse { user: user.into() })),
        Err(SignupError::Expired) => {
            let mut builder = HttpResponse::Unauthorized();
            builder.cookie(expired_cookie(SIGNUP_COOKIE));
            Ok(error_response(builder, EXPIRED))
        }
        Err(SignupError::Database(diesel::result::Error::DatabaseError(
            diesel::result::DatabaseErrorKind::UniqueViolation,
            _,
        ))) => Ok(error_response(
            HttpResponse::Conflict(),
            "That username is already in use.",
        )),
        Err(SignupError::Database(error)) => Err(actix_web::error::ErrorInternalServerError(error)),
    }
}

#[put("/api/auth/me")]
pub async fn update_username(
    pool: web::Data<DbPool>,
    request: HttpRequest,
    input: web::Json<UsernameInput>,
) -> actix_web::Result<HttpResponse> {
    let Some(user_id) = authenticated_user_id(pool.clone(), &request).await? else {
        return Ok(error_response(
            HttpResponse::Unauthorized(),
            "Sign in to change your username.",
        ));
    };
    let username = match normalize_username(&input.username) {
        Ok(username) => username,
        Err(error) => return Ok(error_response(HttpResponse::BadRequest(), error)),
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
        ))) => Ok(error_response(
            HttpResponse::Conflict(),
            "That username is already in use.",
        )),
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
        return Ok(error_response(
            HttpResponse::Unauthorized(),
            "Sign in to delete your account.",
        ));
    };

    web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        diesel::delete(users::table.find(user_id))
            .execute(&mut connection)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;

    Ok(HttpResponse::NoContent()
        .cookie(expired_cookie(SESSION_COOKIE))
        .finish())
}

pub async fn authenticated_user_id(
    pool: web::Data<DbPool>,
    request: &HttpRequest,
) -> actix_web::Result<Option<i64>> {
    let Some(hash) = cookie_hash(request, SESSION_COOKIE) else {
        return Ok(None);
    };
    let now = Utc::now();

    web::block(move || {
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
    .map_err(actix_web::error::ErrorInternalServerError)
}

#[post("/api/auth/logout")]
pub async fn logout(pool: web::Data<DbPool>, request: HttpRequest) -> HttpResponse {
    let session = cookie_hash(&request, SESSION_COOKIE);
    let signup = cookie_hash(&request, SIGNUP_COOKIE);
    if session.is_some() || signup.is_some() {
        let result = web::block(move || {
            let mut connection = pool.get().map_err(|error| error.to_string())?;
            if let Some(hash) = session {
                diesel::delete(sessions::table.filter(sessions::session_hash.eq(hash)))
                    .execute(&mut connection)
                    .map_err(|error| error.to_string())?;
            }
            if let Some(hash) = signup {
                diesel::delete(
                    pending_signups::table.filter(pending_signups::signup_hash.eq(hash)),
                )
                .execute(&mut connection)
                .map_err(|error| error.to_string())?;
            }
            Ok::<_, String>(())
        })
        .await;

        match result {
            Ok(Ok(())) => {}
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

    HttpResponse::Ok()
        .cookie(expired_cookie(SESSION_COOKIE))
        .cookie(expired_cookie(SIGNUP_COOKIE))
        .finish()
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
