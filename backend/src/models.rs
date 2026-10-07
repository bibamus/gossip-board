use chrono::{DateTime, Utc};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::{comments, post_shares, post_tags, posts, tags, users, votes};

#[derive(Debug, Queryable, Selectable, Identifiable, Serialize)]
#[diesel(table_name = users)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct User {
    pub id: i64,
    pub email: String,
    pub username: String,
    pub username_set: bool,
    pub created_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
}

#[derive(Insertable, Deserialize)]
#[diesel(table_name = users)]
pub struct NewUser<'a> {
    pub email: &'a str,
    pub username: &'a str,
    pub username_set: bool,
}

#[derive(Debug, Queryable, Selectable, Identifiable, Associations, Serialize)]
#[diesel(table_name = posts)]
#[diesel(belongs_to(User, foreign_key = author_id))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Post {
    pub id: i64,
    pub author_id: i64,
    pub title: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub image_data: Option<String>,
}

#[derive(Insertable, Deserialize)]
#[diesel(table_name = posts)]
pub struct NewPost<'a> {
    pub author_id: i64,
    pub title: &'a str,
    pub body: &'a str,
    pub image_data: Option<&'a str>,
}

#[derive(Debug, Queryable, Selectable, Identifiable, Associations, Serialize)]
#[diesel(table_name = post_shares)]
#[diesel(belongs_to(Post))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PostShare {
    pub id: i64,
    pub post_id: i64,
    pub shared_by_user_id: i64,
    pub shared_with_user_id: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Insertable, Deserialize)]
#[diesel(table_name = post_shares)]
pub struct NewPostShare {
    pub post_id: i64,
    pub shared_by_user_id: i64,
    pub shared_with_user_id: i64,
}

#[derive(Debug, Queryable, Selectable, Identifiable, Associations, Serialize)]
#[diesel(table_name = comments)]
#[diesel(belongs_to(Post))]
#[diesel(belongs_to(User))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Comment {
    pub id: i64,
    pub post_id: i64,
    pub user_id: i64,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Insertable, Deserialize)]
#[diesel(table_name = comments)]
pub struct NewComment<'a> {
    pub post_id: i64,
    pub user_id: i64,
    pub body: &'a str,
}

#[derive(Debug, Queryable, Selectable, Identifiable, Associations, Serialize)]
#[diesel(table_name = votes)]
#[diesel(belongs_to(Post))]
#[diesel(belongs_to(User))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Vote {
    pub id: i64,
    pub post_id: i64,
    pub user_id: i64,
    pub value: i16,
    pub created_at: DateTime<Utc>,
}

#[derive(Insertable, Deserialize)]
#[diesel(table_name = votes)]
pub struct NewVote {
    pub post_id: i64,
    pub user_id: i64,
    pub value: i16,
}

#[derive(Debug, Queryable, Selectable, Identifiable, Serialize)]
#[diesel(table_name = tags)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub slug: String,
}

#[derive(Insertable, Deserialize)]
#[diesel(table_name = tags)]
pub struct NewTag<'a> {
    pub name: &'a str,
    pub slug: &'a str,
}

#[derive(Debug, Queryable, Selectable, Identifiable, Associations, Serialize)]
#[diesel(table_name = post_tags)]
#[diesel(primary_key(post_id, tag_id))]
#[diesel(belongs_to(Post))]
#[diesel(belongs_to(Tag))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PostTag {
    pub post_id: i64,
    pub tag_id: i64,
}

#[derive(Insertable, Deserialize)]
#[diesel(table_name = post_tags)]
pub struct NewPostTag {
    pub post_id: i64,
    pub tag_id: i64,
}
