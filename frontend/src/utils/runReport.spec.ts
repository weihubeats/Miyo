/**
 * runReport：Markdown 报告构造（快照数据 → 可交付文档）。
 */
import { describe, expect, it } from 'vitest'
import { buildRunReportMarkdown } from '@/utils/runReport'
import type { ScenarioRunResult } from '@/types/miyoApi'

const t = (k: string, params?: Record<string, string | number>) =>
  `«${k}${params ? ':' + JSON.stringify(params) : ''}»`

function makeResult(): ScenarioRunResult {
  return {
    results: [
      {
        endpoint_id: 'e1',
        endpoint_name: '创建订单',
        method: 'POST',
        path: '/shop/orders',
        ok: true,
        status: 200,
        duration_ms: 120,
        outcomes: [
          { description: 'status eq 200', passed: true, expected: '200', actual: '200' },
        ],
        extracted: { orderNo: 'SO-1', payToken: 'pt-9' },
        url: 'http://x/shop/orders',
        step_index: 0,
        response: {
          status: 200,
          headers: [['Authorization', 'Bearer secret-token'], ['Content-Type', 'application/json']],
          body: '{"orderNo":"SO-1"}',
          content_type: 'application/json',
          duration_ms: 120,
          size_bytes: 18,
          truncated: false,
          history_truncated: false,
          request: {
            method: 'POST',
            url: 'http://x/shop/orders',
            url_template: null,
            headers: [['Authorization', 'Bearer secret-token']],
            cookies: [],
            body: '{"sku":"A"}',
            body_omitted: false,
          },
        },
        step_status: 'success',
      },
      {
        endpoint_id: 'e2',
        endpoint_name: '确认支付',
        method: 'POST',
        path: '/shop/pay',
        ok: false,
        status: 400,
        duration_ms: 80,
        request_error: null,
        outcomes: [
          {
            description: 'jsonpath eq SUCCESS',
            passed: false,
            reason: '实际值 x 不等于期望值 SUCCESS',
            expected: 'SUCCESS',
            actual: 'x',
          },
        ],
        extracted: {},
        url: 'http://x/shop/pay',
        step_index: 1,
        step_status: 'failed',
      },
    ],
    cancelled: false,
    stopped_on_failure: false,
    variables: { orderNo: 'SO-1', payToken: 'pt-9', password: 'plain-pwd' },
  }
}

describe('buildRunReportMarkdown', () => {
  it('输出标题/汇总/断言表/快照与脱敏', () => {
    const md = buildRunReportMarkdown({
      scenarioName: '电商主流程',
      runId: 'run-1',
      startedAt: '2026-11-07T01:00:00Z',
      finishedAt: '2026-11-07T01:00:01Z',
      result: makeResult(),
      t,
    })
    // 标题与元信息
    expect(md).toContain('«integration.report.title»')
    expect(md).toContain('电商主流程')
    expect(md).toContain('`run-1`')
    // 汇总：1 过 1 败
    expect(md).toContain('«integration.statusSuccess» 1')
    expect(md).toContain('«integration.statusFailed» 1')
    // 失败步的断言明细（预期/实际）
    expect(md).toContain('SUCCESS')
    expect(md).toContain('| x |')
    // 请求快照 Authorization 脱敏（敏感头不进报告）
    expect(md).not.toContain('secret-token')
    expect(md).toContain('__MIYO_REDACTED__')
    // 响应体保留
    expect(md).toContain('{"orderNo":"SO-1"}')
    // 提取变量
    expect(md).toContain('`{{orderNo}}` = SO-1')
  })

  it('敏感变量池在提取行不泄露（password 走变量池展示由 UI 脱敏，报告只含提取值）', () => {
    const md = buildRunReportMarkdown({
      scenarioName: 'x',
      runId: 'r',
      startedAt: null,
      finishedAt: null,
      result: makeResult(),
      t,
    })
    expect(md).not.toContain('plain-pwd')
  })
})
