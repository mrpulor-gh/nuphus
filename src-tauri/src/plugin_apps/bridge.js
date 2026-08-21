/* Nuphus App Plugin Bridge v1 — 插件侧 SDK（宿主统一版本，经 /plugins-shared/bridge.js 伺服）
 * 协议：docs/plugin-app-system-plan.md §5（postMessage 信封 RPC）。
 * 用法：<script src="/plugins-shared/bridge.js"></script> → window.NuphusBridge
 * 无依赖 / ES2019。v1 显式排除：文件系统、shell、其他插件数据、会话历史。
 */
(function (global) {
  'use strict';

  var VERSION = 1;
  var seq = 0;
  var pending = Object.create(null);   // id -> { resolve, reject }
  var listeners = Object.create(null); // event -> [cb]
  var state = {
    ready: false,
    pluginId: null,
    permissions: [],
    theme: null,
    locale: null,
    hostVersion: null
  };

  function post(envelope) {
    global.parent.postMessage(envelope, '*');
  }

  function call(method, params) {
    var id = 'req-' + (++seq);
    post({ nuphus: VERSION, id: id, type: 'call', method: method, params: params === undefined ? null : params });
    return new Promise(function (resolve, reject) {
      pending[id] = { resolve: resolve, reject: reject };
    });
  }

  function on(event, cb) {
    (listeners[event] = listeners[event] || []).push(cb);
    return function off() {
      var arr = listeners[event];
      if (arr) {
        listeners[event] = arr.filter(function (f) { return f !== cb; });
      }
    };
  }

  function emit(event, payload) {
    (listeners[event] || []).forEach(function (cb) {
      try { cb(payload); } catch (e) { /* 插件回调异常不得中断桥 */ }
    });
  }

  function handleMessage(ev) {
    var msg = ev.data;
    if (!msg || msg.nuphus !== VERSION) return; // 信封版本不符：静默丢弃
    if (msg.type === 'result') {
      var p = pending[msg.id];
      if (!p) return;
      delete pending[msg.id];
      if (msg.ok) p.resolve(msg.payload);
      else p.reject(msg.error || new Error('Nuphus bridge error'));
    } else if (msg.type === 'init') {
      var info = msg.payload || {};
      state.ready = true;
      state.pluginId = info.pluginId || null;
      state.permissions = info.permissions || [];
      state.theme = info.theme || null;
      state.locale = info.locale || null;
      state.hostVersion = info.hostVersion || null;
      emit('init', info);
    } else if (msg.type === 'event') {
      emit(msg.event, msg.payload);
    }
  }

  global.addEventListener('message', handleMessage);

  // 自动 ready 握手：宿主校验 origin + source 绑定 pluginId 后回 init
  post({ nuphus: VERSION, type: 'ready', payload: {} });

  global.NuphusBridge = {
    version: VERSION,
    call: call,
    on: on,
    ready: function (payload) { post({ nuphus: VERSION, type: 'ready', payload: payload || {} }); },
    getState: function () { return state; },
    get pluginId() { return state.pluginId; },
    get permissions() { return state.permissions; },
    get theme() { return state.theme; }
  };
})(window);
