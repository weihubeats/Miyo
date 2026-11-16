/**
 * curlPaste 单测：从整条 cURL 命令里抽出 JSON 正文并排版（不做整条请求导入）。
 */
import { describe, expect, it } from 'vitest'
import { extractCurlBodyJson, splitShellArgs } from '@/utils/curlPaste'

describe('extractCurlBodyJson', () => {
  it('-d 单引号载荷：抽出并美化', () => {
    expect(extractCurlBodyJson(`curl -X POST https://api.x.com/u -d '{"a":1,"b":[1,2]}'`)).toBe(
      '{\n  "a": 1,\n  "b": [\n    1,\n    2\n  ]\n}',
    )
  })

  it('--data 双引号载荷（含转义引号）', () => {
    expect(extractCurlBodyJson('curl --data "{\\"a\\":1}" https://api.x.com')).toBe('{\n  "a": 1\n}')
  })

  it('--json / --data=… 写法都认', () => {
    expect(extractCurlBodyJson(`curl --json '{"a":1}' https://api.x.com`)).toBe('{\n  "a": 1\n}')
    expect(extractCurlBodyJson(`curl --data='{"a":1}' https://api.x.com`)).toBe('{\n  "a": 1\n}')
  })

  it('多个 -d 按 curl 语义用 & 拼接后再解析', () => {
    expect(
      extractCurlBodyJson(`curl -d '{"a":1' -d ',"b":2}' https://api.x.com`),
    ).toBe('{\n  "a": 1,\n  "b": 2\n}')
  })

  it('续行反斜杠 / 换行分隔的参数', () => {
    const cmd = 'curl -X POST \\\n  https://api.x.com \\\n  -d \'{"a":1}\''
    expect(extractCurlBodyJson(cmd)).toBe('{\n  "a": 1\n}')
  })

  it('正文不是 JSON（表单 / 无正文 / 读文件）：返回 null', () => {
    expect(extractCurlBodyJson(`curl -d 'a=1&b=2' https://api.x.com`)).toBeNull()
    expect(extractCurlBodyJson('curl https://api.x.com')).toBeNull()
    expect(extractCurlBodyJson(`curl -d @body.json https://api.x.com`)).toBeNull()
  })

  it('不是 cURL 命令：返回 null', () => {
    expect(extractCurlBodyJson('{"a":1}')).toBeNull()
    expect(extractCurlBodyJson('fetch("https://api.x.com", { body: \'{"a":1}\' })')).toBeNull()
  })
})

describe('splitShellArgs', () => {
  it('引号内空白不切分，引号外反斜杠转义', () => {
    expect(splitShellArgs(`curl -d 'a b' "c d" e\\ f`)).toEqual(['curl', '-d', 'a b', 'c d', 'e f'])
  })

  it('空字符串参数保留（引号成对但内容为空）', () => {
    expect(splitShellArgs(`curl -d '' u`)).toEqual(['curl', '-d', '', 'u'])
  })
})
