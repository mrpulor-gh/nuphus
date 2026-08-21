// ── Nuphus batch_exec helpers ──
// Injected into page context for browser_exec tool.
// Each helper auto-records result in this._results[].
// Supports @N refs (1-based index into interactive element list) and CSS selectors.

(function() {
  if (window.__nuphus) return; // already injected

  // ── Dialog interception ──
  // Override alert/confirm/prompt to auto-dismiss and record
  window.__nuphus_dialog = null;
  var _origAlert = window.alert;
  var _origConfirm = window.confirm;
  var _origPrompt = window.prompt;

  window.alert = function(msg) {
    window.__nuphus_dialog = { type: 'alert', message: String(msg || ''), time: Date.now() };
    window.__nuphus._results.push({ op: 'dialog', type: 'alert', message: String(msg || '').substring(0, 200), success: true, detail: 'auto-dismissed' });
  };
  window.confirm = function(msg) {
    window.__nuphus_dialog = { type: 'confirm', message: String(msg || ''), time: Date.now() };
    window.__nuphus._results.push({ op: 'dialog', type: 'confirm', message: String(msg || '').substring(0, 200), success: true, detail: 'auto-accepted' });
    return true;
  };
  window.prompt = function(msg, def) {
    window.__nuphus_dialog = { type: 'prompt', message: String(msg || ''), time: Date.now() };
    window.__nuphus._results.push({ op: 'dialog', type: 'prompt', message: String(msg || '').substring(0, 200), success: true, detail: 'auto-returned default' });
    return def || '';
  };

  window.__nuphus = {
    _results: [],
    _els: null,
    _els_visible: null,

    // Build cached list of interactive elements (expanded selector set)
    _buildInteractiveList: function(visibleOnly) {
      if (visibleOnly && this._els_visible) return this._els_visible;
      if (!visibleOnly && this._els) return this._els;

      const selector = [
        'a[href]', 'button', 'input:not([type="hidden"])', 'textarea', 'select',
        '[onclick]',
        '[role="button"]', '[role="link"]', '[role="textbox"]', '[role="combobox"]',
        '[role="checkbox"]', '[role="radio"]', '[role="switch"]', '[role="menuitem"]',
        '[role="tab"]', '[role="option"]', '[role="listbox"]', '[role="slider"]',
        '[role="searchbox"]', '[role="spinbutton"]', '[role="togglebutton"]',
        '[tabindex]:not([tabindex="-1"])',
        '[contenteditable="true"]',
      ].join(',');

      let els = Array.from(document.querySelectorAll(selector));

      if (visibleOnly) {
        els = els.filter(function(el) {
          const rect = el.getBoundingClientRect();
          if (rect.width === 0 || rect.height === 0) return false;
          const style = window.getComputedStyle(el);
          if (style.display === 'none' || style.visibility === 'hidden') return false;
          return true;
        });
      }

      if (visibleOnly) {
        this._els_visible = els;
      } else {
        this._els = els;
      }
      return els;
    },

    // Resolve a ref: "@N" → DOM element, or CSS selector → DOM element
    _resolve: function(ref) {
      if (typeof ref === 'string' && ref.startsWith('@')) {
        const idx = parseInt(ref.slice(1), 10) - 1;
        if (isNaN(idx) || idx < 0) return null;
        const els = this._buildInteractiveList(false);
        if (idx >= els.length) return null;
        return els[idx];
      }
      // CSS selector
      return document.querySelector(ref);
    },

    // Visibility check aligned with the Rust-side actionability wait:
    // non-zero bounding rect and not display:none / visibility:hidden.
    _isVisible: function(el) {
      const rect = el.getBoundingClientRect();
      if (rect.width === 0 || rect.height === 0) return false;
      const style = window.getComputedStyle(el);
      return style.display !== 'none' && style.visibility !== 'hidden';
    },

    // Playwright-style auto-wait: poll (~100ms) until the ref resolves to a
    // present AND visible element, or the timeout expires. First resolve uses
    // the cached interactive list (zero overhead on the happy path); during
    // polling the cache is refreshed each step so @N refs track DOM changes.
    _resolveActionable: async function(ref, timeoutMs) {
      const deadline = Date.now() + timeoutMs;
      for (;;) {
        const el = this._resolve(ref);
        if (el && this._isVisible(el)) return el;
        if (Date.now() >= deadline) return null;
        // Refresh cached lists so @N refs see the current DOM while polling.
        this._els = null;
        this._els_visible = null;
        await new Promise(function(r) { setTimeout(r, 100); });
      }
    },

    // ── Public helpers ──

    click: async function(ref, timeoutMs) {
      const timeout = (typeof timeoutMs === 'number' && timeoutMs > 0) ? timeoutMs : 5000;
      const el = await this._resolveActionable(ref, timeout);
      if (!el) {
        const r = { op: 'click', ref: ref, success: false, detail: 'Element not found or not visible within ' + timeout + 'ms: ' + ref + ' (hint: run h.snapshot() to confirm page state)' };
        this._results.push(r);
        return r;
      }
      try {
        if (el.scrollIntoViewIfNeeded) el.scrollIntoViewIfNeeded();
        else el.scrollIntoView({ block: 'center' });
        el.click();
        const r = { op: 'click', ref: ref, success: true, detail: '' };
        this._results.push(r);
        return r;
      } catch(e) {
        const r = { op: 'click', ref: ref, success: false, detail: e.message };
        this._results.push(r);
        throw e;
      }
    },

    fill: async function(ref, text, timeoutMs) {
      const timeout = (typeof timeoutMs === 'number' && timeoutMs > 0) ? timeoutMs : 5000;
      const el = await this._resolveActionable(ref, timeout);
      if (!el) {
        const r = { op: 'fill', ref: ref, success: false, detail: 'Element not found or not visible within ' + timeout + 'ms: ' + ref + ' (hint: run h.snapshot() to confirm page state)' };
        this._results.push(r);
        return r;
      }
      try {
        if (el.scrollIntoViewIfNeeded) el.scrollIntoViewIfNeeded();
        else el.scrollIntoView({ block: 'center' });
        el.focus();
        // Clear existing value
        if (typeof el.value !== 'undefined') {
          el.value = '';
          el.value = text;
        } else if (el.isContentEditable) {
          el.textContent = text;
        }
        el.dispatchEvent(new Event('input', { bubbles: true }));
        el.dispatchEvent(new Event('change', { bubbles: true }));
        const r = { op: 'fill', ref: ref, success: true, text: text.substring(0, 50), detail: '' };
        this._results.push(r);
        return r;
      } catch(e) {
        const r = { op: 'fill', ref: ref, success: false, detail: e.message };
        this._results.push(r);
        throw e;
      }
    },

    scroll: function(px) {
      window.scrollBy(0, px);
      const r = { op: 'scroll', px: px, success: true, detail: '' };
      this._results.push(r);
      return r;
    },

    wait: async function(ms) {
      await new Promise(function(r) { setTimeout(r, ms); });
      const r = { op: 'wait', ms: ms, success: true, detail: '' };
      this._results.push(r);
      return r;
    },

    extract: function(selector) {
      try {
        const el = document.querySelector(selector);
        const text = el ? (el.textContent || '').trim().substring(0, 500) : '';
        const r = { op: 'extract', selector: selector, success: true, text: text, detail: '' };
        this._results.push(r);
        return r;
      } catch(e) {
        const r = { op: 'extract', selector: selector, success: false, detail: e.message };
        this._results.push(r);
        throw e;
      }
    },

    snapshot: function() {
      try {
        const els = this._buildInteractiveList(true);
        const lines = els.map(function(el, i) {
          const tag = el.tagName ? el.tagName.toLowerCase() : '?';
          const text = (el.textContent || el.value || '').trim().substring(0, 40);
          const id = el.id ? '#' + el.id : '';
          const cls = el.className && typeof el.className === 'string'
            ? '.' + el.className.split(' ').filter(function(c) { return c && !c.startsWith('_'); }).join('.')
            : '';
          return '@' + (i+1) + ' <' + tag + id + cls + '> "' + text.replace(/"/g, '\\"') + '"';
        });
        const txt = lines.join('\n');
        const r = { op: 'snapshot', success: true, detail: '', text: txt, count: els.length };
        this._results.push(r);
        return r;
      } catch(e) {
        const r = { op: 'snapshot', success: false, detail: e.message };
        this._results.push(r);
        throw e;
      }
    },

    // Return the last intercepted dialog info (alert/confirm/prompt)
    dialog_info: function() {
      const d = window.__nuphus_dialog;
      const r = d ? { op: 'dialog_info', type: d.type, message: d.message, time: d.time }
                   : { op: 'dialog_info', type: null, message: null };
      this._results.push({ ...r, success: true, detail: '' });
      return r;
    },
  };
})();