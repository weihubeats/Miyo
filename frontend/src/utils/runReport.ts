/**
 * runReport：把一次场景执行导出为 Markdown 测试报告。
 *
 * 数据源 = 运行结果（实时或历史回读，请求/响应均为落库快照，非当前配置
 * 重构）；敏感值沿用落库脱敏口径（历史结果已打码，实时结果的请求快照在
 * 导出时按敏感键名再过滤一次）。
 */
import type { ScenarioRunResult, ScenarioStepResult } from '../types/miyoApi'

/** 敏感键名（与 scenarioVars 同词根，导出报告不再放大明文凭据）。 */
const SENSITIVE = ['password', 'passwd', 'pwd', 'secret', 'token', 'authorization', 'api-key', 'apikey', 'api_key', 'cookie']

/** 步骤状态的可读文本。 */
export function stepStatusText(r: ScenarioStepResult, t: (k: string) => string): string {
  const s = r.step_status
  if (s === 'blocked') return t('integration.statusBlocked')
  if (s === 'skipped') return t('integration.statusSkipped')
  if (r.cancelled) return t('scenario.stepStatusCancelled')
  if (s === 'success' || (!s && r.ok)) return t('integration.statusSuccess')
  if (s === 'failed' || (!s && !r.ok)) return t('integration.statusFailed')
  return t('integration.statusPending')
}

/** 头 / KV 行脱敏。 */
function maskPairs(pairs: readonly (readonly [string, string])[] | undefined): [string, string][] {
  return (pairs ?? []).map(([k, v]) => [
    k,
    SENSITIVE.some((s) => k.toLowerCase().includes(s)) && v ? '__MIYO_REDACTED__' : v,
  ])
}

/** 响应正文预览（超长截断，报告不需要全文）。 */
function bodyPreview(body: string | undefined, max = 4000, t: (k: string, params?: Record<string, string | number>) => string): string {
  const raw = body ?? ''
  if (raw.length <= max) return raw || t('integration.report.bodyEmpty')
  return `${raw.slice(0, max)}\n…${t('integration.report.bodyTruncated', { n: raw.length })}`
}

/** 导出 Markdown 报告。 */
export function buildRunReportMarkdown(input: {
  scenarioName: string
  runId: string
  startedAt: string | null
  finishedAt: string | null
  result: ScenarioRunResult
  t: (k: string, params?: Record<string, string | number>) => string
}): string {
  const { scenarioName, runId, startedAt, finishedAt, result, t } = input
  const out: string[] = []
  const ok = result.results.filter((r) => r.step_status === 'success').length
  const blocked = result.results.filter((r) => r.step_status === 'blocked').length
  const skipped = result.results.filter((r) => r.step_status === 'skipped').length
  const failed = result.results.length - ok - blocked - skipped
  const duration = result.results.reduce((acc, r) => acc + (r.duration_ms ?? 0), 0)

  out.push(`# ${t('integration.report.title')}: ${scenarioName}`)
  out.push('')
  out.push(`- ${t('integration.report.runId')}: \`${runId}\``)
  if (startedAt) out.push(`- ${t('integration.report.startedAt')}: ${startedAt}`)
  if (finishedAt) out.push(`- ${t('integration.report.finishedAt')}: ${finishedAt}`)
  out.push(
    `- ${t('integration.report.summary')}: ${t('integration.statusSuccess')} ${ok} · ${t('integration.statusFailed')} ${failed} · ${t('integration.statusSkipped')} ${skipped + blocked} · ${t('integration.report.duration')}: ${Math.round(duration)}ms`,
  )
  out.push('')

  result.results.forEach((r, i) => {
    const name = r.endpoint_name || `#${i + 1}`
    out.push(`## ${i + 1}. ${name} — ${r.method} ${r.path ?? ''}`)
    out.push('')
    out.push(
      `- ${t('integration.compareStatusCol')}: ${stepStatusText(r, t)}${r.status != null ? ` (HTTP ${r.status})` : ''}`,
    )
    if (r.duration_ms != null) out.push(`- ${t('integration.metaDuration')}: ${Math.round(r.duration_ms)}ms`)
    if (r.loop_runs && r.loop_runs > 1) out.push(`- ${t('integration.loopRunsBadge', { n: r.loop_runs })}`)
    if (r.url) out.push(`- URL: \`${r.url}\``)
    if (r.request_error) out.push(`- ${t('integration.report.error')}: ${r.request_error}`)
    if (r.blocked_reason) out.push(`- ${t('integration.blockedReason')}: ${r.blocked_reason}`)
    out.push('')

    // 断言明细
    if (r.outcomes.length) {
      out.push(`### ${t('integration.report.assertions')}`)
      out.push('')
      out.push(`| ${t('integration.assertNameCol')} | ${t('integration.assertResultCol')} | ${t('integration.assertExpectedCol')} | ${t('integration.assertActualCol')} |`)
      out.push('| --- | --- | --- | --- |')
      for (const o of r.outcomes) {
        out.push(
          `| ${o.description} | ${o.passed ? '✅' : '❌'} | ${o.expected ?? '-'} | ${o.actual ?? '-'} |`,
        )
      }
      out.push('')
    }

    // 变量提取
    const extracted = Object.entries(r.extracted ?? {})
    if (extracted.length) {
      out.push(`### ${t('integration.report.extracted')}`)
      out.push('')
      for (const [k, v] of extracted) {
        out.push(`- \`{{${k}}}\` = ${v}`)
      }
      out.push('')
    }

    // 请求 / 响应快照
    const resp = r.response
    if (resp) {
      const headers = maskPairs(resp.headers)
      out.push(`### ${t('integration.report.request')}`)
      out.push('')
      if (resp.request) {
        const reqHeaders = maskPairs(resp.request.headers)
        if (reqHeaders.length) {
          out.push('```')
          for (const [k, v] of reqHeaders) out.push(`${k}: ${v}`)
          out.push('```')
        }
        if (resp.request.body) {
          out.push('```json')
          out.push(bodyPreview(resp.request.body, 4000, t))
          out.push('```')
        }
      } else {
        out.push(`\`${r.url ?? ''}\``)
      }
      out.push('')
      out.push(`### ${t('integration.report.response')}`)
      out.push('')
      out.push('```')
      out.push(`HTTP ${resp.status}`)
      for (const [k, v] of headers) out.push(`${k}: ${v}`)
      out.push('```')
      out.push('```json')
      out.push(bodyPreview(resp.body, 4000, t))
      out.push('```')
      out.push('')
    }
    out.push('---')
    out.push('')
  })

  return out.join('\n')
}
