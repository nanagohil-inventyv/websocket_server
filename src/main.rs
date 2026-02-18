use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use std::{
    collections::HashMap,
    sync::Arc,
};

use tokio::sync::{mpsc, RwLock};
use std::net::SocketAddr;

use tokio::net::TcpListener;
type Tx = mpsc::UnboundedSender<Message>;

#[derive(Clone)]
struct AppState {
    rooms: Arc<RwLock<HashMap<String, HashMap<String, Tx>>>>,
}

#[tokio::main]
async fn main() {
    let state = AppState {
        rooms: Arc::new(RwLock::new(HashMap::new())),
    };

    let app = Router::new()
        .route("/ws/:room_id", get(ws_handler))
        .with_state(state);

    // println!("Axum WebSocket server running on ws://127.0.0.1:3000");/
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("listening on ws://{}", addr);

    let listener: TcpListener =TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();

}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Path(room_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state, room_id))
}

async fn handle_socket(
    socket: WebSocket,
    state: AppState,
    room_id: String,
) {
    let (mut sender, mut receiver) = socket.split();

    // First message should contain username
    let username = if let Some(Ok(Message::Text(name))) = receiver.next().await {
        name
    } else {
        return;
    };

    let (tx, mut rx) = mpsc::unbounded_channel();

    // Insert user into room
    {
        let mut rooms = state.rooms.write().await;
        let room = rooms.entry(room_id.clone())
            .or_insert_with(HashMap::new);

        room.insert(username.clone(), tx.clone());
    }

    broadcast(
        &state,
        &room_id,
        format!("🔔 {} joined", username),
    ).await;

    // Task for sending messages to this client
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Task for receiving messages
    let state_clone = state.clone();
    let room_clone = room_id.clone();
    let username_clone = username.clone();

    let recv_task = tokio::spawn(async move {
        while let Some(Ok(Message::Text(text))) = receiver.next().await {
            let message = format!("{}: {}", username_clone, text);

            broadcast(&state_clone, &room_clone, message).await;
        }
    });

    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }

    // Clean removal on disconnect
    {
        let mut rooms = state.rooms.write().await;

        if let Some(room) = rooms.get_mut(&room_id) {
            room.remove(&username);

            if room.is_empty() {
                rooms.remove(&room_id);
            }
        }
    }

    broadcast(
        &state,
        &room_id,
        format!("❌ {} left", username),
    ).await;
}

async fn broadcast(
    state: &AppState,
    room_id: &str,
    message: String,
) {
    let rooms = state.rooms.read().await;

    if let Some(room) = rooms.get(room_id) {
        for (_, tx) in room {
            let _ = tx.send(Message::Text(message.clone()));
        }
    }
}
