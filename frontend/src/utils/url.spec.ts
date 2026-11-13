import { describe, expect, it } from 'vitest'
import { splitUrl, protocolFromDomain, stripProtocol, withProtocol, isCurlCommand } from './url'

describe('splitUrl', () => {
  it('完整 URL 拆出路径、查询参数与 origin', () => {
    const r = splitUrl('https://api.x.com/posts?userId=1&page=2')
    expect(r.path).toBe('/posts')
    expect(r.origin).toBe('https://api.x.com')
    expect(r.params).toEqual([
      { key: 'userId', value: '1', enabled: true, description: '', field_type: 'string', required: true, example: '' },
      { key: 'page', value: '2', enabled: true, description: '', field_type: 'string', required: true, example: '' },
    ])
  })

  it('无 scheme 时按 https 补全；无路径时补 /', () => {
    const r = splitUrl('api.x.com')
    expect(r.origin).toBe('https://api.x.com')
    expect(r.path).toBe('/')
    expect(r.params).toEqual([])
  })
})

describe('protocolFromDomain', () => {
  it('识别本地图标的四种协议', () => {
    expect(protocolFromDomain('https://api.x.com')).toBe('https')
    expect(protocolFromDomain('http://localhost:3000')).toBe('http')
    expect(protocolFromDomain('wss://s.x.com/ws')).toBe('wss')
    expect(protocolFromDomain('ws://s.x.com')).toBe('ws')
  })

  it('无 scheme / 变量引用 / 无法识别时回退 https', () => {
    expect(protocolFromDomain('api.x.com')).toBe('https')
    expect(protocolFromDomain('{{base_url}}')).toBe('https')
    expect(protocolFromDomain('')).toBe('https')
  })

  it('大小写不敏感', () => {
    expect(protocolFromDomain('HTTPS://api.x.com')).toBe('https')
    expect(protocolFromDomain('Wss://s.x.com')).toBe('wss')
  })
})

describe('stripProtocol / withProtocol', () => {
  it('stripProtocol 去掉协议前缀，保留其余部分', () => {
    expect(stripProtocol('https://api.x.com/path')).toBe('api.x.com/path')
    expect(stripProtocol('localhost:3000')).toBe('localhost:3000')
    expect(stripProtocol('{{base_url}}')).toBe('{{base_url}}')
  })

  it('withProtocol 替换或补全协议', () => {
    expect(withProtocol('http://api.x.com', 'https')).toBe('https://api.x.com')
    expect(withProtocol('api.x.com', 'https')).toBe('https://api.x.com')
    expect(withProtocol('ws://s.x.com', 'wss')).toBe('wss://s.x.com')
  })
})

describe('isCurlCommand', () => {
  it('识别标准 cURL 命令（含多行续行与大小写）', () => {
    expect(isCurlCommand(`curl https://api.x.com/users`)).toBe(true)
    expect(isCurlCommand(`  CURL -X POST 'https://api.x.com/users' -H 'Content-Type: application/json'`)).toBe(true)
    expect(isCurlCommand(`curl \\\n  -X POST https://api.x.com/users`)).toBe(true)
    expect(isCurlCommand(`curl.exe -X GET https://api.x.com`)).toBe(true)
    expect(isCurlCommand(`/usr/bin/curl https://api.x.com/ping`)).toBe(true)
  })

  it('容忍终端提示符前缀', () => {
    expect(isCurlCommand(`$ curl https://api.x.com/users`)).toBe(true)
    expect(isCurlCommand(`> curl -X GET https://api.x.com`)).toBe(true)
  })

  it('普通 URL / 路径不误判', () => {
    expect(isCurlCommand('')).toBe(false)
    expect(isCurlCommand('https://api.x.com/users')).toBe(false)
    expect(isCurlCommand('/api/v1/users')).toBe(false)
    expect(isCurlCommand('https://api.x.com/curlies')).toBe(false)
    expect(isCurlCommand('curlies are cute')).toBe(false)
  })

  /**
   * 单行 input 输入框会把剪贴板里的换行折成空格（实测 insertText("a\nb") → "a b"），
   * 地址栏拿到的 cURL 可能已在 `curl` 与 `-X` 之间插入了空格、续行反斜杠也被吞掉。
   * 判定必须在「空白折叠后」的文本上做，否则整条命令会被当路径塞进地址栏。
   */
  it('容忍续行被折叠成空格 / 反斜杠残留 / 多余空白的 cURL', () => {
    expect(isCurlCommand('curl   -X POST https://api.x.com/users')).toBe(true)
    expect(isCurlCommand('curl \\ -X POST https://api.x.com/users')).toBe(true)
    expect(isCurlCommand('curl\t-X POST https://api.x.com/users')).toBe(true)
    expect(isCurlCommand('$ curl \\\n        --data-raw \'{"a":1}\' \\\n        https://api.x.com/pay')).toBe(true)
  })
})
