/** 两种 OAuth 登录共用的一次性回环服务；无效 state 不消耗回调。 */
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};

pub(super) struct Callback {
    receiver: oneshot::Receiver<HashMap<String, String>>,
    shutdown: Option<oneshot::Sender<()>>,
    server: JoinHandle<Result<(), std::io::Error>>,
}

impl Callback {
    pub fn start(listener: TcpListener, path: &str, state: String) -> Self {
        let (tx, receiver) = oneshot::channel();
        let sender = Arc::new(Mutex::new(Some(tx)));
        let router = axum::Router::new().route(
            path,
            axum::routing::get(
                move |axum::extract::Query(params): axum::extract::Query<
                    HashMap<String, String>,
                >| {
                    let sender = sender.clone();
                    let state = state.clone();
                    async move {
                        if params.get("state") != Some(&state) {
                            return (
                                axum::http::StatusCode::BAD_REQUEST,
                                "Invalid authorization callback",
                            );
                        }
                        if let Some(tx) = sender.lock().ok().and_then(|mut s| s.take()) {
                            let _ = tx.send(params);
                            (
                                axum::http::StatusCode::OK,
                                "Return to Nuphus to complete sign-in.",
                            )
                        } else {
                            (
                                axum::http::StatusCode::CONFLICT,
                                "This callback has already been used.",
                            )
                        }
                    }
                },
            ),
        );
        let (shutdown, rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await
        });
        Self {
            receiver,
            shutdown: Some(shutdown),
            server,
        }
    }

    pub async fn wait(
        mut self,
        cancel: impl std::future::Future<Output = ()>,
        timeout: Duration,
    ) -> Result<HashMap<String, String>, String> {
        let result = tokio::select! {
            result = tokio::time::timeout(timeout, &mut self.receiver) => match result {
                Ok(Ok(params)) => Ok(params),
                Ok(Err(_)) => Err("OAuth callback interrupted".into()),
                Err(_) => Err("OAuth sign-in timed out".into()),
            },
            _ = cancel => Err("OAuth sign-in cancelled".into()),
        };
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if tokio::time::timeout(Duration::from_secs(1), &mut self.server)
            .await
            .is_err()
        {
            self.server.abort();
        }
        result
    }
}
impl Drop for Callback {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.server.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /**
     * 断言端口在宽限期内确实释放（不要求「同一行」就已空出）。
     *
     * 回环服务停止是异步事实：优雅关停/abort 落地后内核释放 socket 与测试
     * 下一条语句执行之间有一个调度间隙，Linux CI 高并行时偶发一拍滞后，
     * 立即断言重绑成功会闪红（ubuntu-latest 迁 26 后出现过）。语义不变：
     * 端口必须释放——只是给内核 3s；逾期仍绑不上则照常失败。
     */
    async fn assert_port_released(address: std::net::SocketAddr) {
        static GRACE: Duration = Duration::from_secs(3);
        let started = tokio::time::Instant::now();
        loop {
            match TcpListener::bind(address).await {
                Ok(probe) => {
                    drop(probe);
                    return;
                }
                Err(error) => {
                    assert!(
                        started.elapsed() < GRACE,
                        "port {address} not released in time: {error}"
                    );
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        }
    }

    #[tokio::test]
    async fn invalid_state_keeps_waiting_and_callback_is_single_use() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let callback = Callback::start(listener, "/callback", "expected".into());
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("http://{address}/callback");
        assert_eq!(
            client
                .get(&url)
                .query(&[("state", "wrong")])
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
        assert_eq!(
            client
                .get(&url)
                .query(&[("state", "expected"), ("code", "one")])
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        assert_eq!(
            client
                .get(&url)
                .query(&[("state", "expected"), ("code", "two")])
                .send()
                .await
                .unwrap()
                .status(),
            409
        );
        let params = callback
            .wait(std::future::pending(), Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(params["code"], "one");
        assert_port_released(address).await;
    }
    #[tokio::test]
    async fn cancellation_and_timeout_release_the_port() {
        for cancel in [true, false] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let callback = Callback::start(listener, "/callback", "state".into());
            let result = callback
                .wait(
                    async {
                        if !cancel {
                            std::future::pending::<()>().await;
                        }
                    },
                    Duration::from_millis(10),
                )
                .await;
            assert!(result.is_err());
            assert_port_released(address).await;
        }
    }
}
