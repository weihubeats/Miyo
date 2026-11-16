/**
 * BodyPanel 粘贴增强单测：
 * - cURL 命令 → 抽出 JSON 正文并排版（整条命令不落进编辑器）；
 * - raw 的 XML 子类型 → 保守排版；
 * - urlencoded / form-data → query string 拆行（单个 key=value 粘进单元格不拦截）。
 * raw(JSON) 的排版本身由 JsonEditor.paste.spec.ts 覆盖，这里只验面板的接线。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { reactive } from 'vue'
import BodyPanel from '@/domains/endpoint/components/BodyPanel.vue'
import { useLocaleStore } from '@/stores/locale'
import { makeDraft } from '@/testUtils/draftFixture'
import { resetBodyDrafts } from '@/utils/bodyDrafts'
import type { Endpoint, KeyValue, MultipartField } from '@/types/miyoApi'

beforeEach(resetBodyDrafts)

function mountPanel(draft: Endpoint) {
  setActivePinia(createPinia())
  useLocaleStore().setMode('zh')
  return mount(BodyPanel, { props: { draft: reactive(draft) }, attachTo: document.body })
}

/** jsdom 没有 ClipboardEvent 的 clipboardData，用普通事件补一个数据源。 */
function firePaste(el: Element, text: string): Event {
  const ev = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(ev, 'clipboardData', { value: { getData: () => text } })
  el.dispatchEvent(ev)
  return ev
}

const pretty = '{\n  "a": 1\n}'

describe('BodyPanel：cURL 粘贴抽正文', () => {
  it('粘整条 cURL 到 raw(JSON)：落库的是抽出的正文（已排版）', async () => {
    const draft = makeDraft()
    draft.request.body = { mode: 'json', raw: '' }
    const w = mountPanel(draft)
    const ta = w.find('textarea').element

    const ev = firePaste(ta, `curl -X POST https://api.x.com -d '{"a":1}'`)
    await w.vm.$nextTick()

    expect(ev.defaultPrevented).toBe(true)
    expect((draft.request.body as { raw: string }).raw).toBe(pretty)
    expect(ta.value).toBe(pretty)
    w.unmount()
  })

  it('cURL 里没有 JSON 正文：不落 curl 原文（拦住并提示）', async () => {
    const draft = makeDraft()
    draft.request.body = { mode: 'json', raw: '{"keep":true}' }
    const w = mountPanel(draft)
    const ta = w.find('textarea').element

    const ev = firePaste(ta, 'curl https://api.x.com')
    await w.vm.$nextTick()

    expect(ev.defaultPrevented).toBe(true)
    expect((draft.request.body as { raw: string }).raw).toBe('{"keep":true}')
    w.unmount()
  })
})

describe('BodyPanel：XML 子类型粘贴排版', () => {
  it('压缩 XML 被排版为多行；混合内容保持原文', async () => {
    const draft = makeDraft()
    draft.request.body = { mode: 'text', raw: '' }
    draft.request.headers = [
      { key: 'Content-Type', value: 'application/xml', enabled: true, description: '', field_type: 'string', required: true, example: '' },
    ]
    const w = mountPanel(draft)
    const ta = w.find('textarea').element

    const ev = firePaste(ta, '<root><a>1</a></root>')
    // textarea 的 raw 有 150ms 防抖镜像，失焦刷出
    w.find('textarea').element.dispatchEvent(new Event('blur'))
    await w.vm.$nextTick()

    expect(ev.defaultPrevented).toBe(true)
    expect(ta.value).toBe('<root>\n  <a>1</a>\n</root>')
    expect((draft.request.body as { raw: string }).raw).toBe('<root>\n  <a>1</a>\n</root>')
    w.unmount()
  })
})

describe('BodyPanel：urlencoded / form-data 粘贴拆行', () => {
  it('urlencoded：一行 query string 拆成多行字段', async () => {
    const draft = makeDraft()
    draft.request.body = { mode: 'urlencoded', fields: [] }
    const w = mountPanel(draft)

    firePaste(w.find('.panel').element, 'a=1&b=two')
    await w.vm.$nextTick()

    const fields = (draft.request.body as { fields: KeyValue[] }).fields
    expect(fields.map((f) => [f.key, f.value])).toEqual([
      ['a', '1'],
      ['b', 'two'],
    ])
    w.unmount()
  })

  it('urlencoded：同名键就地更新，新键追加', async () => {
    const draft = makeDraft()
    draft.request.body = {
      mode: 'urlencoded',
      fields: [{ key: 'a', value: 'old', enabled: true, description: '', field_type: 'string', required: true, example: '' }],
    }
    const w = mountPanel(draft)

    firePaste(w.find('.panel').element, '?a=1&b=2')
    await w.vm.$nextTick()

    const fields = (draft.request.body as { fields: KeyValue[] }).fields
    expect(fields.map((f) => [f.key, f.value])).toEqual([
      ['a', '1'],
      ['b', '2'],
    ])
    w.unmount()
  })

  it('单个 key=value 粘进单元格：不拦截，用户只想填这一格', async () => {
    const draft = makeDraft()
    draft.request.body = {
      mode: 'urlencoded',
      fields: [{ key: 'a', value: '', enabled: true, description: '', field_type: 'string', required: true, example: '' }],
    }
    const w = mountPanel(draft)
    const cell = w.findAll('input')[0].element

    const ev = firePaste(cell, 'b=2')
    await w.vm.$nextTick()

    expect(ev.defaultPrevented).toBe(false)
    expect((draft.request.body as { fields: KeyValue[] }).fields.map((f) => f.key)).toEqual(['a'])
    w.unmount()
  })

  it('form-data：拆成文本字段行', async () => {
    const draft = makeDraft()
    draft.request.body = { mode: 'multipart', fields: [] }
    const w = mountPanel(draft)

    firePaste(w.find('.panel').element, 'a=1&b=2')
    await w.vm.$nextTick()

    const fields = (draft.request.body as { fields: MultipartField[] }).fields
    expect(fields.map((f) => [f.key, f.value, f.value_type])).toEqual([
      ['a', '1', 'text'],
      ['b', '2', 'text'],
    ])
    w.unmount()
  })
})
