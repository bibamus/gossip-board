use std::collections::HashMap;

use actix_web::{get, post, web, HttpRequest, HttpResponse};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    auth::authenticated_user_id,
    db::DbPool,
    models::{NewPostTag, NewTag, Post, Tag},
    posts::visible_posts,
    schema::{post_tags, posts, tags},
};

#[derive(Serialize)]
pub(crate) struct TaggedPost {
    #[serde(flatten)]
    pub post: Post,
    pub tags: Vec<Tag>,
}

#[derive(Deserialize)]
pub struct TagInput {
    name: String,
}

#[derive(Deserialize)]
pub struct TagQuery {
    #[serde(default)]
    catalog: bool,
}

pub(crate) fn normalize_tag(value: &str) -> Result<String, &'static str> {
    if !value.chars().all(|character| {
        character.is_ascii_alphanumeric() || character.is_whitespace() || character == '-'
    }) {
        return Err("Topics may contain letters, numbers, spaces, and hyphens.");
    }
    let name = value
        .split(|character: char| character.is_whitespace() || character == '-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    if name.is_empty() || name.len() > 64 {
        return Err("Topic names must contain between 1 and 64 characters.");
    }
    Ok(name)
}

pub(crate) fn normalize_tags(values: &[String]) -> Result<Vec<String>, &'static str> {
    let mut names = values
        .iter()
        .map(|value| normalize_tag(value))
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    names.dedup();
    Ok(names)
}

fn ensure_tag(connection: &mut PgConnection, name: &str) -> QueryResult<Tag> {
    let slug = name.replace(' ', "-");
    diesel::insert_into(tags::table)
        .values(NewTag { name, slug: &slug })
        .on_conflict(tags::slug)
        .do_update()
        .set(tags::name.eq(name))
        .returning(Tag::as_returning())
        .get_result::<Tag>(connection)
}

pub(crate) fn replace_post_tags(
    connection: &mut PgConnection,
    post_id: i64,
    names: &[String],
) -> QueryResult<()> {
    diesel::delete(post_tags::table.filter(post_tags::post_id.eq(post_id))).execute(connection)?;
    for name in names {
        let tag = ensure_tag(connection, name)?;
        diesel::insert_into(post_tags::table)
            .values(NewPostTag {
                post_id,
                tag_id: tag.id,
            })
            .execute(connection)?;
    }
    Ok(())
}

fn with_tags(connection: &mut PgConnection, posts: Vec<Post>) -> QueryResult<Vec<TaggedPost>> {
    let ids = posts.iter().map(|post| post.id).collect::<Vec<_>>();
    let rows = post_tags::table
        .inner_join(tags::table)
        .filter(post_tags::post_id.eq_any(ids))
        .order(tags::name.asc())
        .select((post_tags::post_id, Tag::as_select()))
        .load::<(i64, Tag)>(connection)?;
    let mut by_post: HashMap<i64, Vec<Tag>> = HashMap::new();
    for (post_id, tag) in rows {
        by_post.entry(post_id).or_default().push(tag);
    }
    Ok(posts
        .into_iter()
        .map(|post| TaggedPost {
            tags: by_post.remove(&post.id).unwrap_or_default(),
            post,
        })
        .collect())
}

pub(crate) fn tagged_post(connection: &mut PgConnection, post: Post) -> QueryResult<TaggedPost> {
    let tags = post_tags::table
        .inner_join(tags::table)
        .filter(post_tags::post_id.eq(post.id))
        .order(tags::name.asc())
        .select(Tag::as_select())
        .load::<Tag>(connection)?;
    Ok(TaggedPost { post, tags })
}

pub(crate) fn visible_tagged_posts(
    connection: &mut PgConnection,
    user_id: i64,
    topic: Option<&str>,
) -> QueryResult<Vec<TaggedPost>> {
    let mut query = visible_posts(user_id);
    if let Some(name) = topic {
        let matching_posts = post_tags::table
            .inner_join(tags::table)
            .filter(tags::name.eq(name))
            .select(post_tags::post_id);
        query = query.filter(posts::id.eq_any(matching_posts));
    }
    let posts = query
        .order((posts::created_at.desc(), posts::id.desc()))
        .select(Post::as_select())
        .load::<Post>(connection)?;
    with_tags(connection, posts)
}

#[get("/api/tags")]
pub async fn list_tags(
    pool: web::Data<DbPool>,
    request: HttpRequest,
    query: web::Query<TagQuery>,
) -> actix_web::Result<HttpResponse> {
    let Some(user_id) = authenticated_user_id(pool.clone(), &request).await? else {
        return Ok(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Sign in to view topics."
        })));
    };
    let result = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        if query.catalog {
            return tags::table
                .order(tags::name.asc())
                .select(Tag::as_select())
                .load::<Tag>(&mut connection)
                .map_err(|error| error.to_string());
        }
        visible_topics(&mut connection, user_id).map_err(|error| error.to_string())
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;
    Ok(HttpResponse::Ok().json(result))
}

pub(crate) fn visible_topics(connection: &mut PgConnection, user_id: i64) -> QueryResult<Vec<Tag>> {
    let visible_ids = visible_posts(user_id).select(posts::id);
    let tag_ids = post_tags::table
        .filter(post_tags::post_id.eq_any(visible_ids))
        .select(post_tags::tag_id);
    tags::table
        .filter(tags::id.eq_any(tag_ids))
        .order(tags::name.asc())
        .select(Tag::as_select())
        .load::<Tag>(connection)
}

#[post("/api/tags")]
pub async fn create_tag(
    pool: web::Data<DbPool>,
    request: HttpRequest,
    input: web::Json<TagInput>,
) -> actix_web::Result<HttpResponse> {
    let Some(_) = authenticated_user_id(pool.clone(), &request).await? else {
        return Ok(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Sign in to create topics."
        })));
    };
    let name = match normalize_tag(&input.name) {
        Ok(name) => name,
        Err(error) => {
            return Ok(HttpResponse::BadRequest().json(serde_json::json!({ "error": error })));
        }
    };
    let result = web::block(move || {
        let mut connection = pool.get().map_err(|error| error.to_string())?;
        ensure_tag(&mut connection, &name).map_err(|error| error.to_string())
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;
    Ok(HttpResponse::Ok().json(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_deduplicates_topics() {
        assert_eq!(
            normalize_tag("  Office-- NEWS \t"),
            Ok("office news".to_owned())
        );
        assert_eq!(
            normalize_tags(&["News".to_owned(), " NEWS ".to_owned()]),
            Ok(vec!["news".to_owned()])
        );
        assert!(normalize_tag(" --- ").is_err());
        assert!(normalize_tag(&"a".repeat(65)).is_err());
        assert!(normalize_tag("news,rumors").is_err());
        assert!(normalize_tags(&[" ".to_owned()]).is_err());
        assert_eq!(normalize_tag(&"a".repeat(64)), Ok("a".repeat(64)));
    }

    #[actix_web::test]
    async fn topic_routes_require_authentication() {
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
                .service(list_tags)
                .service(create_tag)
                .service(crate::posts::update_post_tags)
                .service(crate::posts::list_posts),
        )
        .await;
        let requests = [
            test::TestRequest::get().uri("/api/tags").to_request(),
            test::TestRequest::post()
                .uri("/api/tags")
                .set_json(serde_json::json!({ "name": "News" }))
                .to_request(),
            test::TestRequest::put()
                .uri("/api/posts/1/tags")
                .set_json(serde_json::json!({ "tags": ["News"] }))
                .to_request(),
            test::TestRequest::get()
                .uri("/api/posts?tag=news")
                .to_request(),
        ];
        for request in requests {
            let result = test::call_service(&app, request).await;
            assert_eq!(result.status(), StatusCode::UNAUTHORIZED);
        }
    }
}
