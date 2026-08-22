// splash.js — 监听后端 `splash:progress` 事件驱动 splash 进度条与状态文案。
//
// 载荷 { pct?: number, text: string }：
//   pct 为 0..=100 时渲染定宽进度；pct 缺失（null）时切到不定宽滑动条。
// 事件走 Tauri IPC（app.emit），不受本页 CSP `script-src 'self'` 限制。
(function () {
  'use strict';

  var hint = document.querySelector('.hint');
  var bar = document.getElementById('bar');
  var fill = document.getElementById('barFill');
  var skipWrap = document.getElementById('skipWrap');
  var skipBtn = document.getElementById('skipBtn');

  function setIndeterminate(on) {
    bar.classList.toggle('indeterminate', on);
    if (on) fill.style.width = '30%';
  }

  function setPct(pct) {
    setIndeterminate(false);
    var p = Math.max(0, Math.min(100, Math.round(pct)));
    fill.style.width = p + '%';
  }

  function setText(text) {
    if (!hint || hint.textContent === text) return;
    hint.style.opacity = '0';
    setTimeout(function () {
      hint.textContent = text;
      hint.style.opacity = '1';
    }, 120);
  }

  // 下载超过 30s 仍未结束 → 出现「后台下载」按钮。
  // 缓存路径下 splash 秒关，定时器无副作用；只有真正卡在下载/启动才触发。
  var skipTimer = setTimeout(function () {
    if (skipWrap) skipWrap.hidden = false;
  }, 30000);

  function start() {
    var tauri = window.__TAURI__;
    // withGlobalTauri 未注入（异常环境）：静态文案 + 不定宽条继续转，不抛错。
    if (!tauri || !tauri.event) {
      setText('正在启动…');
      return;
    }
    tauri.event
      .listen('splash:progress', function (ev) {
        var d = ev.payload || {};
        if (typeof d.pct === 'number') {
          setPct(d.pct);
        } else {
          setIndeterminate(true);
        }
        if (d.text) setText(d.text);
      })
      .catch(function () {
        // listen 注册失败：静默回退到静态状态
      });

    // 点击「后台下载」→ 后端关闭 splash + 显示主窗口，下载继续（主界面 HUD 提示）
    if (skipBtn) {
      skipBtn.addEventListener('click', function () {
        skipBtn.disabled = true;
        if (skipWrap) skipWrap.hidden = true;
        setText('正在打开应用…');
        if (tauri.core) {
          tauri.core
            .invoke('splash_skip_download')
            .catch(function () {
              skipBtn.disabled = false;
              if (skipWrap) skipWrap.hidden = false;
            });
        }
      });
    }
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start);
  } else {
    start();
  }
})();