#![forbid(unsafe_code)]

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, RwLock},
};

use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, Query, State},
    http::{request::Parts, StatusCode},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use axum_macros::{debug_handler, FromRef};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use tracing_subscriber::{prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

#[tokio::main]
async fn main() {
    // tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "example_http_interface=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // store data into volatile memory
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    setup_sqlite(&pool).await;

    // State
    let app_state = AppState {
        pool: Arc::new(SharedDB(pool)),
        user: Arc::new(SharedUser::default()),
    };
    // HTTP interface
    let app = Router::new()
        .route("/identifiers", post(create_identifier).get(get_identifier))
        .route(
            "/products",
            post(create_product)
                .get(get_product)
                .put(modify_product)
                .delete(delete_product),
        )
        .with_state(app_state);

    // run it with hyper on localhost:3000
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn setup_sqlite(pool: &SqlitePool) {
    sqlx::query(
        r#"
        CREATE TABLE product (
        owner text,
        name text,
        description text
            )"#,
    )
    .execute(pool)
    .await
    .unwrap();
    tracing::info!("Sqlite setup complete");
}

#[debug_handler(state = AppState)]
async fn create_identifier(
    State(user): State<Arc<SharedUser>>,
    Query(username): Query<CreateUser>,
) -> impl IntoResponse {
    let id = Uuid::new_v4();
    let mut usermap = user.0.write().unwrap();

    tracing::info!("{} assigned to {}", username.username, id);
    usermap.insert(id, username.username);

    (StatusCode::OK, Json(id.hyphenated().to_string()))
}

#[debug_handler(state = AppState)]
async fn create_product(
    id: RequiredUserId,
    State(pool): State<Arc<SharedDB>>,
    Json(payload): Json<Product>,
) -> impl IntoResponse {
    // check if product already exists
    sqlx::query(
        r#"
            INSERT INTO 
            product (owner, name, description)
            VALUES (?1, ?2, ?3)
        "#,
    )
    .bind(&id.0.to_string())
    .bind(&payload.name)
    .bind(&payload.description)
    .execute(&pool.0)
    .await
    .unwrap();

    tracing::info!("Inserted product for {}", id.0);

    (
        StatusCode::CREATED,
        Json(Product {
            name: payload.name,
            description: payload.description,
        }),
    )
}

#[debug_handler(state = AppState)]
async fn get_identifier(
    id: RequiredUserId,
    State(user): State<Arc<SharedUser>>,
) -> impl IntoResponse {
    let list = user.0.read().unwrap();
    let result = list.get_key_value(&id.0).unwrap();
    tracing::info!("Information provided about: {}", result.1);

    (
        StatusCode::FOUND,
        Json(User {
            id: result.0.to_string(),
            username: result.1.to_owned(),
        }),
    )
}

#[debug_handler(state = AppState)]
async fn get_product(id: RequiredUserId, State(pool): State<Arc<SharedDB>>) -> impl IntoResponse {
    let product =
        sqlx::query_as::<_, Product>("SELECT name, description FROM product WHERE owner = ?1")
            .bind(id.0.to_string())
            .fetch_one(&pool.0)
            .await;
    let Ok(product) = product else {
        return (StatusCode::NOT_FOUND, Json(Product::default()))
    };
    (StatusCode::FOUND, Json(product))
}

#[debug_handler(state = AppState)]
async fn delete_product(
    id: RequiredUserId,
    State(pool): State<Arc<SharedDB>>,
) -> impl IntoResponse {
    let product = sqlx::query("DELETE FROM product WHERE owner = ?1")
        .bind(id.0.to_string())
        .execute(&pool.0)
        .await
        .unwrap();

    let true = product.rows_affected() == 1 else {
        return StatusCode::NOT_FOUND;
    };
    StatusCode::NO_CONTENT
}

#[debug_handler(state = AppState)]
async fn modify_product(
    id: RequiredUserId,
    State(pool): State<Arc<SharedDB>>,
    Json(payload): Json<ModifyProduct>,
) -> impl IntoResponse {
    let product =
        sqlx::query_as::<_, Product>("SELECT name, description FROM product WHERE owner = ?1")
            .bind(id.0.to_string())
            .fetch_one(&pool.0)
            .await;
    let Ok(product) = product else {
        return StatusCode::NOT_FOUND
    };

    let new_name = payload.name.unwrap_or(product.name);
    let new_description = payload.description.unwrap_or(product.description);

    let product = sqlx::query("UPDATE product SET name = ?1, description = ?2 WHERE owner = ?3")
        .bind(new_name)
        .bind(new_description)
        .bind(id.0.to_string())
        .execute(&pool.0)
        .await
        .unwrap();

    let true = product.rows_affected() == 1 else {
        // this should never happen; INSERT error
        return StatusCode::INTERNAL_SERVER_ERROR;
    };

    StatusCode::NO_CONTENT
}

struct RequiredUserId(Uuid);

#[async_trait]
impl<S> FromRequestParts<S> for RequiredUserId
where
    Arc<SharedUser>: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let user = Arc::<SharedUser>::from_ref(state);

        let id = parts
            .headers
            .get("uuid")
            .and_then(|id| id.to_str().ok())
            .ok_or((StatusCode::FORBIDDEN, "Please pass your identifier"))?;

        verify_uuid(id, user).await
    }
}

async fn verify_uuid(
    uuid: &str,
    user: Arc<SharedUser>,
) -> Result<RequiredUserId, (StatusCode, &'static str)> {
    let Ok(uuid) = Uuid::parse_str(uuid) else {
        return Err((StatusCode::FORBIDDEN,"Invalid identifier"))
    };

    let usermap = user.0.read().unwrap();
    usermap
        .get(&uuid)
        .ok_or((StatusCode::FORBIDDEN, "Invalid identifier!"))
        .map(|_| RequiredUserId(uuid))
}

#[derive(Deserialize, Serialize, Default, FromRow)]
struct Product {
    name: String,
    description: String,
}

#[derive(Deserialize)]
struct ModifyProduct {
    name: Option<String>,
    description: Option<String>,
}

#[derive(Serialize, Default)]
struct User {
    id: String,
    username: String,
}

#[derive(Deserialize)]
struct CreateUser {
    username: String,
}

#[derive(Default)]
struct SharedUser(RwLock<HashMap<Uuid, String>>);

struct SharedDB(SqlitePool);

#[derive(Clone, FromRef)]
struct AppState {
    user: Arc<SharedUser>,
    pool: Arc<SharedDB>,
}
