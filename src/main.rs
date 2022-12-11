#![allow(dead_code, unused_variables)]
use axum::{routing::get, Router};
use sqlx::SqlitePool;
use tracing_subscriber::{prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "example_http_interface=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // store data into volatile memory
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();

    let app = Router::new().route("/", get(|| async { "Hello, World!" }));

    // run it with hyper on localhost:3000
    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}

#[cfg(feature = "v7")]
async fn get_ident() {
    let uuid = Uuid::now_v7();
    "{uuid}"
}
