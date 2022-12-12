#![forbid(unsafe_code)]
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, RwLock},
};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
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

    let shared_users = Arc::new(RwLock::new(UserState {
        users: HashMap::new(),
    }));

    // store data into volatile memory
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();

    // HTTP interface
    let app = Router::new()
        .route("/identifiers", post(create_identifier))
        .route("/identifiers/:uuid", get(get_identifier))
        .with_state(shared_users);

    // run it with hyper on localhost:3000
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn create_identifier(
    State(state): State<SharedUserState>,
    Query(user): Query<CreateUser>,
) -> impl IntoResponse {
    let id = Uuid::new_v4();
    let user = user.username;

    let mut w = state.write().unwrap();
    tracing::info!("{} assigned to {}", user, id);
    w.users.insert(id, user);

    (StatusCode::OK, Json(id.hyphenated().to_string()))
}

async fn get_identifier(
    State(state): State<SharedUserState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uuid = Uuid::parse_str(&id);
    let Ok(uuid) = uuid else {
        return (StatusCode::NOT_FOUND, Json(User::default()))
    };

    let list = state.read().unwrap();
    let result = list.users.get_key_value(&uuid).unwrap();
    tracing::info!("Information provided about: {}", result.1);

    (
        StatusCode::FOUND,
        Json(User {
            id: result.0.to_string(),
            username: result.1.to_string(),
        }),
    )
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
struct UserState {
    users: HashMap<Uuid, String>,
}

// store username -> id
type SharedUserState = Arc<RwLock<UserState>>;
