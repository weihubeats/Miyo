/**
 * urlBar：地址栏文本编辑纯函数回归（自 EndpointEditor 抽出）。
 * 锁定粘贴分流的取词与归一化语义——这些函数曾酿成「/routes routes」系列 bug。
 */
import { describe, expect, it, vi } from 'vitest'
import {
  normalizeCurlCommand,
  pickPathToken,
  setFieldValueAt,
} from './urlBar'

describe('pickPathToken', () => {
  it('裸路径原样保留', () => {
    expect(pickPathToken('/routes')).toBe('/routes')
  })

  it('文档片段取含 / 的那一段', () => {
    expect(pickPathToken('POST /routes')).toBe('/routes')
    expect(pickPathToken('/routes 创建路由')).toBe('/routes')
    expect(pickPathToken('api /api/v1/x 用户列表')).toBe('/api/v1/x')
  })

  it('首段只剩 origin 的完整地址与路径段拼回（主机不丢）', () => {
    expect(pickPathToken('http://h:1 /routes')).toBe('http://h:1/routes')
    expect(pickPathToken('https://api.x.com /v1/users?id=1')).toBe('https://api.x.com/v1/users?id=1')
  })

  it('都没有 / 时取首段（重复标签去噪）', () => {
    expect(pickPathToken('routes routes')).toBe('routes')
  })

  it('纯空白返回空串（调用方丢弃）', () => {
    expect(pickPathToken('   \n\t ')).toBe('')
  })
})

describe('normalizeCurlCommand', () => {
  it('去终端提示符', () => {
    expect(normalizeCurlCommand('$ curl https://x.com')).toBe('curl https://x.com')
    expect(normalizeCurlCommand('# curl https://x.com')).toBe('curl https://x.com')
    expect(normalizeCurlCommand('> curl https://x.com')).toBe('curl https://x.com')
  })

  it('shell 续行折成空格', () => {
    expect(normalizeCurlCommand('curl -X POST \\\n  https://x.com \\\n  -H a:b')).toBe(
      'curl -X POST    https://x.com    -H a:b',
    )
  })

  it('首尾空白裁剪', () => {
    expect(normalizeCurlCommand('  curl https://x.com  ')).toBe('curl https://x.com')
  })
})

describe('setFieldValueAt', () => {
  it('写值 + 光标定位 + 触发 input 事件（兜底路径不进撤销栈）', () => {
    const el = document.createElement('input')
    const fired = vi.fn()
    el.addEventListener('input', fired)
    setFieldValueAt(el, '/routes', 3)
    expect(el.value).toBe('/routes')
    expect(el.selectionStart).toBe(3)
    expect(fired).toHaveBeenCalledTimes(1)
  })

  it('非文本输入定位失败不抛错（类型限制跳过光标）', () => {
    const el = document.createElement('input')
    el.type = 'number'
    expect(() => setFieldValueAt(el, '1', 1)).not.toThrow()
    expect(el.value).toBe('1')
  })
})
