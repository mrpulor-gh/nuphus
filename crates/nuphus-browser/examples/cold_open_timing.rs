//! 冷启动计时探针 —— 走与 `browser_show_window` 完全相同的代码路径，把
//! 「点击 → 浏览器窗口就绪」拆成 预检 / 冷启 / 建页置前 三段并打印毫秒。
//!
//! 为什么留着它：入口慢在哪一段是能被证伪的事，不该靠外部猜测（网络嗅探、
//! PowerShell 计时）间接推断——那些探针量到的不是本进程的路径。
//!
//! 运行（必须 release，debug 目标被宿主进程锁着）：
//!   cargo run --release -p nuphus-browser --example cold_open_timing
//!
//! 副作用：真实拉起一个 Chrome 窗口（Nuphus 自己的 profile），结束前关闭。
//! 若已有同 profile 的实例在跑，`launch` 会走 attach（打印出来的是热路径）。

use std::time::Instant;

use nuphus_browser::{runtime, BrowserClient};

fn main() {
    let t0 = Instant::now();
    let mut client = BrowserClient::new().expect("BrowserClient::new");
    println!(
        "new()                          {:>6} ms",
        t0.elapsed().as_millis()
    );

    let total = runtime().block_on(async {
        let t = Instant::now();
        client.launch(false).await.expect("launch(headed)");
        println!(
            "launch(headed) [预检+冷启]     {:>6} ms",
            t.elapsed().as_millis()
        );

        let t = Instant::now();
        client.bring_to_front().await.expect("bring_to_front");
        println!(
            "bring_to_front [建页+置前]     {:>6} ms",
            t.elapsed().as_millis()
        );

        let t = Instant::now();
        let url = client.current_url().await.unwrap_or_default();
        println!(
            "current_url                    {:>6} ms  ({url})",
            t.elapsed().as_millis()
        );

        t0.elapsed().as_millis()
    });

    println!("──────────────────────────────────────────");
    println!("TOTAL（窗口就绪，同命令口径）  {total:>6} ms");

    // 收尾：探针不该留下悬挂的浏览器进程。
    runtime().block_on(async {
        let _ = client.close().await;
    });
}
