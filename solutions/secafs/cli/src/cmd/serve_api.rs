use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::rpc::backend::FuseMountBackend;
use crate::rpc::codec::{encode_response, parse_request};
use crate::rpc::methods::{dispatch, State};
use crate::rpc::types::Response;

pub struct ServeApiArgs {
    pub socket: PathBuf,
    pub pg_url: String,
    pub mount_root: PathBuf,
}

pub async fn run(args: ServeApiArgs) -> anyhow::Result<()> {
    // Ensure parent dirs exist.
    if let Some(parent) = args.socket.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create socket parent {parent:?}"))?;
    }
    std::fs::create_dir_all(&args.mount_root)
        .with_context(|| format!("create mount root {:?}", args.mount_root))?;

    // Clean up any stale socket.
    let _ = std::fs::remove_file(&args.socket);

    let backend = FuseMountBackend::new(&args.pg_url, 4).await?;
    let state = Arc::new(State {
        backend: backend.clone(),
        mount_root: args.mount_root.clone(),
        mounts: tokio::sync::Mutex::new(Default::default()),
    });

    let listener = UnixListener::bind(&args.socket)
        .with_context(|| format!("bind socket {:?}", args.socket))?;
    eprintln!("[secafs serve api] listening on {}", args.socket.display());

    // SIGTERM/SIGINT cleanup: unmount all, remove socket, exit.
    let socket_path_cleanup = args.socket.clone();
    let state_shutdown = state.clone();
    tokio::spawn(async move {
        let mut sigterm = tokio::signal::unix::signal(
            tokio::signal::unix::SignalKind::terminate(),
        )
        .expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
        eprintln!("[secafs serve api] shutting down, unmounting all");
        let ids: Vec<String> = state_shutdown
            .mounts
            .lock()
            .await
            .keys()
            .cloned()
            .collect();
        for id in ids {
            let _ = state_shutdown.backend.unmount(&id).await;
        }
        let _ = std::fs::remove_file(&socket_path_cleanup);
        std::process::exit(0);
    });

    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_conn(stream, state).await {
                eprintln!("[secafs serve api] conn error: {e}");
            }
        });
    }
}

async fn serve_conn(stream: UnixStream, state: Arc<State>) -> anyhow::Result<()> {
    let (r, mut w) = stream.into_split();
    let mut reader = BufReader::new(r);
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let resp = match parse_request(trimmed) {
            Err(e) => encode_response(&Response::error(json!(null), -32700, &e, None)),
            Ok(req) => match dispatch(&state, &req.method, req.params).await {
                Ok(result) => encode_response(&Response::success(
                    req.id.unwrap_or(json!(null)),
                    result,
                )),
                Err((code, msg)) => encode_response(&Response::error(
                    req.id.unwrap_or(json!(null)),
                    code,
                    &msg,
                    None,
                )),
            },
        };
        w.write_all(resp.as_bytes()).await?;
        w.write_all(b"\n").await?;
    }
    Ok(())
}
