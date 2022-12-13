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
use sqlx::SqlitePool;
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
        .route("/identifiers", post(create_identifier))
        //.route("/identifiers/:uuid", get(get_identifier))
        .route("/products", post(create_product))
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

#[debug_handler(state=AppState)]
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

#[debug_handler(state=AppState)]
async fn create_product(
    id: UserID,
    State(pool): State<Arc<SharedDB>>,
    Json(payload): Json<Product>,
) -> impl IntoResponse {
    sqlx::query(
        r#"
            INSERT INTO 
            product (owner, name, description)
            VALUES (?1, ?2, ?3)
        "#,
    )
    .bind(&id.0.hyphenated().to_string())
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

// async fn get_identifier(
//     State(state): State<SharedUserState>,
//     Path(id): Path<String>,
// ) -> impl IntoResponse {
//     let uuid = Uuid::parse_str(&id);
//     let Ok(uuid) = uuid else {
//         return (StatusCode::NOT_FOUND, Json(User::default()))
//     };

//     let list = state.read().unwrap();
//     let result = list.users.get_key_value(&uuid).unwrap();
//     tracing::info!("Information provided about: {}", result.1);

//     (
//         StatusCode::FOUND,
//         Json(User {
//             id: result.0.to_string(),
//             username: result.1.to_string(),
//         }),
//     )
// }

struct UserID(Uuid);

#[async_trait]
impl<S> FromRequestParts<S> for UserID
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
) -> Result<UserID, (StatusCode, &'static str)> {
    let Ok(uuid) = Uuid::parse_str(uuid) else {
        return Err((StatusCode::FORBIDDEN,"Please generate your identifier first"))
    };

    let usermap = user.0.read().unwrap();
    usermap
        .get(&uuid)
        .ok_or((StatusCode::FORBIDDEN, "Invalid identifier!"))
        .map(|_| UserID(uuid))
}

#[derive(Deserialize, Serialize, Default)]
struct Product {
    name: String,
    description: String,
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
