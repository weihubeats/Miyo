/**
 * codeImport 单测：各语言 HTTP 客户端代码片段 → CurlParsed。
 * 覆盖常见写法：JS fetch/axios、Python requests、Java OkHttp、Go net/http。
 */
import { beforeEach, describe, expect, it } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { detectLang, parseCodeSnippet } from './codeImport'
import { useLocaleStore } from '../stores/locale'

beforeEach(() => {
  setActivePinia(createPinia())
  // 错误文案断言锁定中文（jsdom 默认语言为英文，跟随系统会解析出英文）
  useLocaleStore().setMode('zh')
})

describe('detectLang', () => {
  it('识别各语言特征', () => {
    expect(detectLang("curl -X POST 'https://a.com' -d '{}'")).toBe('curl')
    expect(
      detectLang('OkHttpClient client = new OkHttpClient();\nRequest r = new Request.Builder().url("https://a.com").build();'),
    ).toBe('java')
    expect(detectLang('URL url = new URL("https://a.com");\nHttpURLConnection conn = (HttpURLConnection) url.openConnection();')).toBe('java')
    expect(detectLang("import requests\nresp = requests.get('https://a.com')")).toBe('python')
    expect(detectLang("fetch('https://a.com')")).toBe('javascript')
    expect(detectLang("await axios.post('https://a.com', {})")).toBe('javascript')
    expect(detectLang('req, err := http.NewRequest("GET", "https://a.com", nil)')).toBe('go')
    expect(detectLang('console.log("hello")')).toBeNull()
  })
})

describe('parseCodeSnippet: JavaScript fetch', () => {
  it('POST + headers + JSON.stringify body', () => {
    const src = `
const resp = await fetch('https://api.example.com/users', {
  method: 'POST',
  headers: {
    'Content-Type': 'application/json',
    'X-Token': 'abc',
  },
  body: JSON.stringify({ name: 'alice', age: 18 }),
});
`
    const parsed = parseCodeSnippet('javascript', src)
    expect(parsed.url).toBe('https://api.example.com/users')
    expect(parsed.method).toBe('POST')
    expect(parsed.headers).toEqual([
      { key: 'Content-Type', value: 'application/json', enabled: true, description: '', field_type: 'string', required: true, example: '' },
      { key: 'X-Token', value: 'abc', enabled: true, description: '', field_type: 'string', required: true, example: '' },
    ])
    expect(parsed.body).toEqual({ mode: 'json', raw: '{"name":"alice","age":18}' })
  })

  it('GET 无 body', () => {
    const src = `fetch('https://api.example.com/items?page=2')`
    const parsed = parseCodeSnippet('javascript', src)
    expect(parsed.method).toBe('GET')
    expect(parsed.body).toBeNull()
  })
})

describe('parseCodeSnippet: axios', () => {
  it('axios.post(url, 对象字面量) 提取 body', () => {
    const src = `
const { data } = await axios.post('https://api.example.com/orders', { sku: 'A-1', count: 2 }, {
  headers: { Authorization: 'Bearer tok' },
});
`
    const parsed = parseCodeSnippet('javascript', src)
    expect(parsed.method).toBe('POST')
    expect(parsed.body).toEqual({ mode: 'json', raw: '{"sku":"A-1","count":2}' })
    expect(parsed.headers).toEqual([{ key: 'Authorization', value: 'Bearer tok', enabled: true, description: '', field_type: 'string', required: true, example: '' }])
  })
})

describe('parseCodeSnippet: Python requests', () => {
  it('post + headers + json dict', () => {
    const src = `
import requests

url = "https://api.example.com/users"
headers = {"Content-Type": "application/json", "Accept": "application/json"}
resp = requests.post(url, headers=headers, json={"name": "bob"})
`
    const parsed = parseCodeSnippet('python', src)
    expect(parsed.url).toBe('https://api.example.com/users')
    expect(parsed.method).toBe('POST')
    expect(parsed.headers).toHaveLength(2)
    expect(parsed.body).toEqual({ mode: 'json', raw: '{"name":"bob"}' })
  })

  it('data= 表单串 → urlencoded 字段', () => {
    const src = `
import requests
resp = requests.post("https://api.example.com/login", headers={"Content-Type": "application/x-www-form-urlencoded"}, data="user=tom&pwd=a b")
`
    const parsed = parseCodeSnippet('python', src)
    expect(parsed.body?.mode).toBe('urlencoded')
    expect(parsed.body).toMatchObject({
      mode: 'urlencoded',
      fields: [
        { key: 'user', value: 'tom' },
        { key: 'pwd', value: 'a b' },
      ],
    })
  })
})

describe('parseCodeSnippet: Java OkHttp', () => {
  it('post + header + RequestBody.create(优先 MediaType)', () => {
    const src = `
OkHttpClient client = new OkHttpClient();
MediaType mediaType = MediaType.parse("application/json");
RequestBody body = RequestBody.create(mediaType, "{\\"name\\":\\"carol\\"}");
Request request = new Request.Builder()
  .url("https://api.example.com/users")
  .post(body)
  .header("Authorization", "Bearer jt")
  .build();
`
    const parsed = parseCodeSnippet('java', src)
    expect(parsed.url).toBe('https://api.example.com/users')
    expect(parsed.method).toBe('POST')
    expect(parsed.headers).toContainEqual({ key: 'Content-Type', value: 'application/json', enabled: true, description: '', field_type: 'string', required: true, example: '' })
    expect(parsed.headers).toContainEqual({ key: 'Authorization', value: 'Bearer jt', enabled: true, description: '', field_type: 'string', required: true, example: '' })
    expect(parsed.body).toEqual({ mode: 'json', raw: '{"name":"carol"}' })
  })

  it('HttpURLConnection setRequestProperty + write body', () => {
    const src = `
URL url = new URL("https://api.example.com/echo");
HttpURLConnection conn = (HttpURLConnection) url.openConnection();
conn.setRequestMethod("PUT");
conn.setRequestProperty("Content-Type", "text/plain");
conn.setDoOutput(true);
conn.getOutputStream().write("hello java".getBytes(StandardCharsets.UTF_8));
`
    const parsed = parseCodeSnippet('java', src)
    expect(parsed.method).toBe('PUT')
    expect(parsed.body).toEqual({ mode: 'text', raw: 'hello java' })
  })
})

describe('parseCodeSnippet: Go net/http', () => {
  it('NewRequest + Header.Set + bytes.NewBufferString', () => {
    const src = `
package main

import ("bytes"; "net/http")

func main() {
  payload := []byte("{\\"k\\":\\"v\\"}")
  req, _ := http.NewRequest("POST", "https://api.example.com/items", bytes.NewBuffer(payload))
  req.Header.Set("Content-Type", "application/json")
  req.Header.Set("X-Trace", "t1")
}
`
    const parsed = parseCodeSnippet('go', src)
    expect(parsed.url).toBe('https://api.example.com/items')
    expect(parsed.method).toBe('POST')
    expect(parsed.headers).toEqual([
      { key: 'Content-Type', value: 'application/json', enabled: true, description: '', field_type: 'string', required: true, example: '' },
      { key: 'X-Trace', value: 't1', enabled: true, description: '', field_type: 'string', required: true, example: '' },
    ])
  })

  it('strings.NewReader 文本 body', () => {
    const src = `
req, _ := http.NewRequest("POST", "https://api.example.com/t", strings.NewReader("name=go&x=1"))
`
    const parsed = parseCodeSnippet('go', src)
    expect(parsed.method).toBe('POST')
    expect(parsed.body).toEqual({ mode: 'text', raw: 'name=go&x=1' })
  })
})

describe('parseCodeSnippet: Rust reqwest', () => {
  it('post + header + body 字面量', () => {
    const src = `
let client = reqwest::Client::new();
let res = client.post("https://api.example.com/orders")
    .header("Content-Type", "application/json")
    .header("X-Token", "abc")
    .body("{\\"amount\\":99}")
    .send()
    .await?;
`
    const parsed = parseCodeSnippet('rust', src)
    expect(parsed.url).toBe('https://api.example.com/orders')
    expect(parsed.method).toBe('POST')
    expect(parsed.headers).toEqual([
      { key: 'Content-Type', value: 'application/json', enabled: true, description: '', field_type: 'string', required: true, example: '' },
      { key: 'X-Token', value: 'abc', enabled: true, description: '', field_type: 'string', required: true, example: '' },
    ])
    expect(parsed.body).toEqual({ mode: 'json', raw: '{"amount":99}' })
  })

  it('bearer_auth 还原为 Bearer 头，basic_auth 还原为 Basic 认证', () => {
    const src = `
let res = client.get("https://api.example.com/me")
    .bearer_auth("tok-1")
    .send()
    .await?;
`
    const parsed = parseCodeSnippet('rust', src)
    expect(parsed.method).toBe('GET')
    expect(parsed.headers).toEqual([
      { key: 'Authorization', value: 'Bearer tok-1', enabled: true, description: '', field_type: 'string', required: true, example: '' },
    ])
    const src2 = `client.get("https://api.example.com/x").basic_auth("u", "p").send().await?;`
    const parsed2 = parseCodeSnippet('rust', src2)
    expect(parsed2.auth).toEqual({ type: 'basic', username: 'u', password: 'p' })
  })

  it('Method::PUT 显式形式 + .json 对象', () => {
    const src = `
let res = client.request(reqwest::Method::PUT, "https://api.example.com/u/1")
    .json(&serde_json::json!({"name": "a"}))
    .send()
    .await?;
`
    const parsed = parseCodeSnippet('rust', src)
    expect(parsed.method).toBe('PUT')
    expect(parsed.body).toEqual({ mode: 'json', raw: '{"name":"a"}' })
  })
})

describe('parseCodeSnippet: PHP', () => {
  it('curl_setopt URL + headers + POSTFIELDS', () => {
    const src = `
$ch = curl_init();
curl_setopt($ch, CURLOPT_URL, "https://api.example.com/pay");
curl_setopt($ch, CURLOPT_HTTPHEADER, array("Content-Type: application/json", "X-Sign: s"));
curl_setopt($ch, CURLOPT_POSTFIELDS, "{\\"order\\":1}");
curl_setopt($ch, CURLOPT_CUSTOMREQUEST, "POST");
`
    const parsed = parseCodeSnippet('php', src)
    expect(parsed.url).toBe('https://api.example.com/pay')
    expect(parsed.method).toBe('POST')
    expect(parsed.headers).toEqual([
      { key: 'Content-Type', value: 'application/json', enabled: true, description: '', field_type: 'string', required: true, example: '' },
      { key: 'X-Sign', value: 's', enabled: true, description: '', field_type: 'string', required: true, example: '' },
    ])
    expect(parsed.body).toEqual({ mode: 'json', raw: '{"order":1}' })
  })

  it('Guzzle post + headers/json', () => {
    const src = `
$client = new Client();
$res = $client->post("https://api.example.com/g", [
    'headers' => ['X-A' => 'b'],
    'json' => ['k' => 'v'],
]);
`
    const parsed = parseCodeSnippet('php', src)
    expect(parsed.url).toBe('https://api.example.com/g')
    expect(parsed.method).toBe('POST')
    expect(parsed.headers).toEqual([
      { key: 'X-A', value: 'b', enabled: true, description: '', field_type: 'string', required: true, example: '' },
    ])
    expect(parsed.body).toEqual({ mode: 'json', raw: '{"k":"v"}' })
  })
})

describe('parseCodeSnippet: 失败路径', () => {
  it('找不到 URL 时抛出中文错误', () => {
    expect(() => parseCodeSnippet('python', 'resp = requests.get(BASE_URL)')).toThrow(/URL/)
  })

  it('curl 走后端，前端直接拒绝', () => {
    expect(() => parseCodeSnippet('curl', 'curl https://a.com')).toThrow(/cURL/)
  })
})
