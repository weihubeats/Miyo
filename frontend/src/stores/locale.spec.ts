/**
 * locale 单测：模式切换 / 跟随系统 / 插值 / 缺键回退。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { translate, useLocaleStore } from '@/stores/locale'

beforeEach(() => {
  setActivePinia(createPinia())
  localStorage.clear()
})

describe('locale store', () => {
  it('默认跟随系统：中文系统解析为 zh', () => {
    vi.spyOn(window.navigator, 'language', 'get').mockReturnValue('zh-CN')
    const store = useLocaleStore()
    expect(store.mode).toBe('system')
    expect(store.resolved).toBe('zh')
    expect(store.t('common.save')).toBe('保存')
  })

  it('英文系统解析为 en，切换偏好即时生效并持久化', () => {
    vi.spyOn(window.navigator, 'language', 'get').mockReturnValue('en-US')
    const store = useLocaleStore()
    expect(store.resolved).toBe('en')
    expect(store.t('common.save')).toBe('Save')
    store.setMode('zh')
    expect(store.resolved).toBe('zh')
    expect(store.t('common.save')).toBe('保存')
    expect(localStorage.getItem('miyo.locale.mode')).toBe('zh')
    expect(document.documentElement.getAttribute('lang')).toBe('zh-CN')
  })

  it('插值替换命名参数', () => {
    expect(translate('zh', 'settings.timeoutSaved', { v: 30 })).toBe('已保存：请求超时 30 秒')
    expect(translate('en', 'settings.timeoutSaved', { v: 30 })).toBe('Saved: request timeout 30s')
  })

  it('缺键回退中文，缺中文返回键名', () => {
    expect(translate('en', 'settings.timeout')).toBe('Request timeout')
    expect(translate('en', 'no.such.key')).toBe('no.such.key')
  })

  it('中英文字典键完全一致', async () => {
    const { zh } = await import('@/i18n/zh')
    const { en } = await import('@/i18n/en')
    expect(Object.keys(en).sort()).toEqual(Object.keys(zh).sort())
  })
})
