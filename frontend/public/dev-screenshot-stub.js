/*
 * Miyo 截图/验收辅助数据桩（dev-only）。
 *
 * 用途：在纯浏览器（非 Tauri）环境打开 vite dev server 时，提供 `?miyo-e2e=1`
 * 查询参数加载本文件，把 Tauri IPC 换成固定 fixture，使界面可用固定数据渲染，
 * 供三视口截图与布局验收。生产构建不执行、不引用（index.html 里的加载分支
 * 以查询参数为闸；本文件仅是 public 静态资源，从未被应用代码 import）。
 *
 * 数据全部为虚构样例，不携带真实业务信息。
 */
(function () {
  if (window.__TAURI_INTERNALS__) return;
  var now = Date.now();
  var iso = function (m) { return new Date(now - m * 60000).toISOString() };
  function ep(id, name, method, path) {
    return {
      id: id, project_id: 'p1', folder_id: null, name: name, method: method, path: path,
      description: '', status: 'released', sort_order: 0,
      request: { base_url: '', params: [], path_variables: [], headers: [], body: { mode: 'none' }, auth: null, timeout_ms: null, follow_redirects: true },
      created_at: iso(60 * 24 * 20), updated_at: iso(60 * 8),
    };
  }
  function scen(id, name, cat, ok, fail) {
    return {
      id: id, project_id: 'p1', name: name, description: '', category: cat, sort_order: 0,
      steps: [
        { endpoint_id: 'e1', enabled: true, name: '创建订单', extract: [{ name: 'orderNo', path: '$.data.order_no' }], assertions: [] },
        { endpoint_id: 'e2', enabled: true, name: '查询订单', extract: [], assertions: [{ type: 'status', path: null, op: 'eq', expected: 200 }] },
      ],
      setup: [], teardown: [], teardown_enabled: true, data_sets: [], script: null,
      created_at: iso(60 * 24 * 10), updated_at: iso(60 * 5),
      last_run: { id: 'r-' + id, scenario_id: id, finished_at: iso(30), duration_ms: 1240 + ok * 100, total_steps: ok + fail, ok_count: ok, fail_count: fail, skipped_count: 0, cancelled: false, stopped_on_failure: false },
    };
  }
  var FIXTURES = {
    get_projects: [
      { id: 'p1', name: '支付网关', description: '收单 / 对账 / 清算链路', variables: {}, team_id: 't1', created_at: iso(60 * 24 * 30), updated_at: iso(12) },
      { id: 'p2', name: '用户中心', description: '注册登录 / 会员体系 / 权限', variables: {}, team_id: 't1', created_at: iso(60 * 24 * 20), updated_at: iso(180) },
      { id: 'p3', name: '消息推送', description: '', variables: {}, team_id: 't2', created_at: iso(60 * 24 * 9), updated_at: iso(60 * 26) },
    ],
    list_teams: [
      { id: 't1', name: '核心交易', created_at: iso(60 * 24 * 40), updated_at: iso(60) },
      { id: 't2', name: '增长与触达', created_at: iso(60 * 24 * 15), updated_at: iso(60 * 24) },
    ],
    get_active_project: {
      id: 'p1', name: '支付网关', description: '收单 / 对账 / 清算链路', variables: {}, team_id: 't1', created_at: iso(60 * 24 * 30), updated_at: iso(12),
    },
    get_active_project_id: 'p1',
    list_project_stats: [
      { project_id: 'p1', endpoint_count: 24, scenario_count: 3, request_count: 156, ok_count: 147, fail_count: 9, latest_method: 'POST', latest_path: '/v1/refund' },
      { project_id: 'p2', endpoint_count: 13, scenario_count: 1, request_count: 89, ok_count: 71, fail_count: 18, latest_method: 'GET', latest_path: '/api/users' },
      { project_id: 'p3', endpoint_count: 5, scenario_count: 0, request_count: 0, ok_count: 0, fail_count: 0, latest_method: null, latest_path: null },
    ],
    list_project_quality: [
      { project_id: 'p1', scenarios_total: 3, scenarios_ran: 3, scenario_last_ok: 2, scenario_last_fail: 1, scenario_last_failed_at: iso(35), cases_total: 12, cases_success: 9, cases_failed: 2, cases_untested: 1, case_last_failed_at: iso(95) },
      { project_id: 'p2', scenarios_total: 1, scenarios_ran: 0, scenario_last_ok: 0, scenario_last_fail: 0, scenario_last_failed_at: null, cases_total: 4, cases_success: 0, cases_failed: 0, cases_untested: 4, case_last_failed_at: null },
      { project_id: 'p3', scenarios_total: 0, scenarios_ran: 0, scenario_last_ok: 0, scenario_last_fail: 0, scenario_last_failed_at: null, cases_total: 0, cases_success: 0, cases_failed: 0, cases_untested: 0, case_last_failed_at: null },
    ],
    list_recent_activities: [
      { id: 'a1', project_id: 'p1', project_name: '支付网关', endpoint_id: null, endpoint_name: '退款', method: 'POST', url: 'https://api.pay.example.com/v1/refund', status: 200, duration_ms: 128, created_at: iso(4) },
      { id: 'a2', project_id: 'p1', project_name: '支付网关', endpoint_id: null, endpoint_name: '对账单下载', method: 'GET', url: 'https://api.pay.example.com/v1/statements?date=2026-11-13', status: 502, duration_ms: 4210, created_at: iso(18) },
      { id: 'a3', project_id: 'p2', project_name: '用户中心', endpoint_id: null, endpoint_name: '用户列表', method: 'GET', url: 'https://api.user.example.com/api/users?page=1', status: 200, duration_ms: 66, created_at: iso(52) },
      { id: 'a4', project_id: 'p2', project_name: '用户中心', endpoint_id: null, endpoint_name: '登录', method: 'POST', url: 'https://api.user.example.com/api/login', status: 401, duration_ms: 39, created_at: iso(140) },
      { id: 'a5', project_id: 'p3', project_name: '消息推送', endpoint_id: null, endpoint_name: '模板查询', method: 'GET', url: 'https://api.msg.example.com/templates', status: null, duration_ms: null, created_at: iso(300) },
      { id: 'a6', project_id: 'p1', project_name: '支付网关', endpoint_id: null, endpoint_name: '创建订单', method: 'POST', url: 'https://api.pay.example.com/v1/orders', status: 201, duration_ms: 212, created_at: iso(420) },
    ],
    list_endpoints: [
      ep('e1', '创建订单', 'POST', '/v1/orders'),
      ep('e2', '查询订单', 'GET', '/v1/orders/{order_no}'),
      ep('e3', '退款', 'POST', '/v1/refund'),
    ],
    list_scenarios: [
      scen('s1', '下单到退款全链路', '核心链路', 6, 1),
      scen('s2', '对账单核验', '核对', 4, 0),
      scen('s3', '白名单场景', '', 3, 3),
    ],
    list_scenario_runs: [],
    list_test_cases: [{
      id: 'c1', request_id: 'e1', name: '创建订单-正向', category: '正向', folder: '下单链路',
      method: 'POST', url_path: '/v1/orders',
      path_variables: [], params: [{ key: 'channel', value: 'alipay', enabled: true, description: '', field_type: 'string', required: true, example: '' }],
      headers: [], body_type: 'json', body_content: '{"amount": 100}',
      assertions: [{ type: 'status', path: null, op: 'eq', expected: 200 }],
      extractors: [], auth: null, timeout_ms: null, disable_cookies: false,
      sort_order: 0, last_run_status: 'Success', last_run_at: iso(40), last_run_ok: 3, last_run_fail: 0,
      created_at: iso(60 * 24 * 6),
    }],
    get_scenario: null,
    list_test_case_runs: [],
    list_environments: [],
    list_folders: [],
    get_http_timeout_ms: 300000,
  };
  window.__E2E_IPC_LOG = [];
  window.addEventListener('error', function (e) {
    (window.__E2E_ERRORS = window.__E2E_ERRORS || []).push('ERR: ' + ((e.error && e.error.stack) || e.message));
  });
  window.addEventListener('unhandledrejection', function (e) {
    (window.__E2E_ERRORS = window.__E2E_ERRORS || []).push('REJ: ' + ((e.reason && e.reason.stack) || String(e.reason)));
  });
  window.__E2E_IPC_LOG = [];
  // 模拟事件回放：按事件名精确派发（真实 Tauri 运行时经 plugin:event|listen
  // 注册 handler，此处记录 handler → 事件名映射后定向广播）。
  var eventNames = Object.create(null);
  function fireTauriEvent(name, payload) {
    Object.keys(eventNames).forEach(function (hid) {
      if (eventNames[hid] !== name) return;
      var cb = window['_' + hid];
      if (typeof cb === 'function') {
        try { cb({ event: name, id: 0, payload: payload }) } catch { /* 单个监听器异常不扩散 */ }
      }
    });
  }
  function replayWs(connId) {
    fireTauriEvent('miyo:ws-event', { connection_id: connId, kind: 'state', state: 'open' });
    var frames = [
      ['in', 'text', '{"type":"welcome","server":"miyo-echo","ts":1731560400000}'],
      ['in', 'text', '{"type":"presence","users":["alice","bob","cw"]}'],
      ['out', 'text', 'hello from miyo'],
      ['in', 'text', '{"type":"echo","payload":"hello from miyo"}'],
      ['in', 'ping', 'aGVsbG8='],
      ['in', 'binary', 'eyJiaW4iOiB0cnVlLCAibmFtZSI6ICJwYXlsb2FkLTAxIn0='],
      ['out', 'text', '{"action":"subscribe","channel":"orders"}'],
      ['in', 'text', '{"type":"subscribed","channel":"orders","seq":42}'],
    ];
    frames.forEach(function (f, i) {
      setTimeout(function () {
        // f = [dir, kind, text]；载荷形状对齐 WsEventPayload（flat，snake_case）
        fireTauriEvent('miyo:ws-event', { connection_id: connId, kind: 'message', direction: f[0], frame: f[1], text: f[2] });
      }, 260 + i * 220);
    });
  }
  function replaySse(connId) {
    fireTauriEvent('miyo:sse-event', { connection_id: connId, kind: 'open' });
    var chunks = [
      'event: message\ndata: {"step":1,"text":"realtime 实时流验收"}\nid: 1001\n\n',
      'event: message\ndata: {"step":2,"text":"第二帧，含中文与 emoji"}\nid: 1002\n\n',
      'event: tick\ndata: {"t": 1731560400}\nid: 1003\n\n',
    ];
    chunks.forEach(function (c, i) {
      setTimeout(function () {
        fireTauriEvent('miyo:sse-event', { connection_id: connId, kind: 'chunk', chunk: c });
      }, 300 + i * 260);
    });
  }
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main', windowLabel: 'main' } },
    plugins: {},
    transformCallback: function (cb) {
      var id = Math.floor(Math.random() * 1e9);
      window['_' + id] = cb;
      return id;
    },
    invoke: function (cmd, args) {
      var bare = String(cmd);
      var idx = bare.indexOf('|');
      if (idx >= 0 && bare.slice(0, idx) === 'plugin:miyo') bare = bare.slice(idx + 1);
      window.__E2E_IPC_LOG.push(bare);
      if (bare.indexOf('plugin:event|') === 0) {
        // listen/unlisten：记录 handler id → 事件名（定向广播用）
        if (bare === 'listen' || bare.endsWith('|listen')) {
          try { eventNames[String(args.handler)] = String(args.event) } catch { /* 忽略 */ }
        }
        return Promise.resolve(Math.floor(Math.random() * 1e9));
      }
      if (bare === 'list_scenario_summaries') return Promise.resolve(JSON.parse(JSON.stringify(FIXTURES.list_scenarios)));
      if (bare === 'ws_connect') {
        var id = 'ws-' + Math.floor(Math.random() * 1e6);
        setTimeout(function () { replayWs(id) }, 200);
        return Promise.resolve(id);
      }
      if (bare === 'sse_connect') {
        var sid = 'sse-' + Math.floor(Math.random() * 1e6);
        setTimeout(function () { replaySse(sid) }, 200);
        return Promise.resolve(sid);
      }
      if (bare === 'ws_send') {
        // echo：文本帧回显一帧进向（帧日志能看到收发对）
        var sendArgs = args || {};
        var cid = sendArgs.connectionId || sendArgs.connection_id;
        if (sendArgs.frame === 'text') {
          setTimeout(function () {
            fireTauriEvent('miyo:ws-event', { connection_id: cid, kind: 'message', direction: 'in', frame: 'text', text: 'echo: ' + sendArgs.payload });
          }, 350);
        }
        return Promise.resolve(null);
      }
      if (bare in FIXTURES) return Promise.resolve(JSON.parse(JSON.stringify(FIXTURES[bare])));
      return Promise.resolve(null);
    },
  };
})();
