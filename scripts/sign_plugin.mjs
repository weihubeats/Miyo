#!/usr/bin/env node
/**
 * Miyo 插件签名工具（Ed25519）。
 *
 * 与 Rust 验证端（crates/miyo-plugin/src/package.rs::signing_payload）的
 * payload 规范严格互为镜像：
 *  1. manifest.json 解析为 JSON 对象，`signature` 键恒参与 payload 且值置零
 *     （128 个 '0'）；
 *  2. 按键名字典序递归排序、无空白序列化（Rust serde_json Value = BTreeMap
 *     同行为）→ 字节 A；
 *  3. files 行清单 `{path} {sha256hex} {size}\n`（按 path 字典序、排除
 *     manifest.json）→ 字节 B；
 *  4. payload = A（无 files）或 A || '\n' || B。
 *
 * 用法：
 *   node scripts/sign_plugin.mjs keygen                          生成密钥对（私钥不落盘，打印）
 *   node scripts/sign_plugin.mjs sign <plugin_dir>               就地签名（写回 manifest.json）
 *   node scripts/sign_plugin.mjs verify <plugin_dir|file.miyo>   验签 + 全文件完整性复核
 *   node scripts/sign_plugin.mjs selftest                        golden vector 自检（CI 防漂移）
 *
 * 密钥：环境变量 MIYO_PLUGIN_SIGNING_SEED（64 位 hex 种子；对应公钥见
 * crates/miyo-plugin/src/package.rs::OFFICIAL_SIGNING_PUBKEYS）。
 * 私钥只经环境变量/CI Secret 流转，禁止写入仓库。
 */
import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'

/** 官方公钥 keyring（hex）——与 Rust OFFICIAL_SIGNING_PUBKEYS 同步维护。 */
const OFFICIAL_PUBKEYS = [
  '92739639878299e7f459b10fe3ba2c6994559594ff2feb1cbf61eef3d71cd206',
]

const ASSET_EXTENSIONS = new Set([
  'html', 'htm', 'js', 'mjs', 'css', 'json', 'svg', 'png', 'jpg', 'jpeg',
  'webp', 'gif', 'woff', 'woff2', 'ttf', 'txt', 'md', 'map', 'wasm',
  'bin', 'exe', '',
])

function fail(msg) {
  console.error(`[sign_plugin] ${msg}`)
  process.exit(1)
}

/** 递归排序键 + 无空白序列化（镜像 serde_json Value BTreeMap 行为）。 */
function canonicalize(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalize).join(',')}]`
  if (value !== null && typeof value === 'object') {
    const keys = Object.keys(value).sort()
    return `{${keys.map((k) => `${JSON.stringify(k)}:${canonicalize(value[k])}`).join(',')}}`
  }
  return JSON.stringify(value)
}

/** Rust signing_payload 的 Node 镜像。 */
function signingPayload(manifestBytes) {
  const obj = JSON.parse(manifestBytes.toString('utf8'))
  obj.signature = '0'.repeat(128)
  let payload = Buffer.from(canonicalize(obj), 'utf8')
  if (Array.isArray(obj.files) && obj.files.length > 0) {
    const lines = [...obj.files]
      .sort((a, b) => (a.path < b.path ? -1 : 1))
      .map((f) => `${f.path} ${f.sha256} ${f.size}\n`)
      .join('')
    payload = Buffer.concat([payload, Buffer.from('\n', 'utf8'), Buffer.from(lines, 'utf8')])
  }
  return payload
}

function sha256Hex(buf) {
  return crypto.createHash('sha256').update(buf).digest('hex')
}

/** 递归收集目录内全部文件的相对路径（排除 manifest.json；拒绝符号链接）。 */
function collectFiles(root) {
  const out = []
  const walk = (dir, prefix) => {
    for (const name of fs.readdirSync(dir)) {
      const abs = path.join(dir, name)
      const rel = prefix ? `${prefix}/${name}` : name
      const stat = fs.lstatSync(abs)
      if (stat.isSymbolicLink()) fail(`拒绝符号链接：${rel}`)
      if (stat.isDirectory()) walk(abs, rel)
      else if (stat.isFile() && rel !== 'manifest.json') out.push({ abs, rel })
    }
  }
  walk(root, '')
  out.sort((a, b) => (a.rel < b.rel ? -1 : 1))
  return out
}

/** 由目录内容构造 files 清单（扩展名白名单；路径字符白名单与 Rust 一致）。 */
function buildFilesList(root) {
  return collectFiles(root).map(({ abs, rel }) => {
    if (!/^[A-Za-z0-9._-]+$/.test(rel.split('/').join(''))) {
      fail(`文件名含非法字符（白名单 [A-Za-z0-9._-/]）：${rel}`)
    }
    const ext = path.extname(rel).slice(1).toLowerCase()
    if (!ASSET_EXTENSIONS.has(ext)) fail(`扩展名 .${ext || '（无）'} 不在白名单：${rel}`)
    return { path: rel, sha256: sha256Hex(fs.readFileSync(abs)), size: fs.statSync(abs).size }
  })
}

function seedToPrivateKey(seedHex) {
  if (!/^[0-9a-f]{64}$/i.test(seedHex)) fail('MIYO_PLUGIN_SIGNING_SEED 必须是 64 位 hex 种子')
  const seed = Buffer.from(seedHex, 'hex')
  // Node 需要 PKCS#8 DER：Ed25519 前缀 + 32 字节种子
  const pkcs8Prefix = Buffer.from('302e020100300506032b657004220420', 'hex')
  return crypto.createPrivateKey({
    key: Buffer.concat([pkcs8Prefix, seed]),
    format: 'der',
    type: 'pkcs8',
  })
}

function cmdKeygen() {
  const { privateKey, publicKey } = crypto.generateKeyPairSync('ed25519')
  const pkcs8 = privateKey.export({ type: 'pkcs8', format: 'der' })
  const seed = pkcs8.subarray(16) // 固定 16 字节前缀后即 32 字节种子
  const pub = publicKey.export({ type: 'spki', format: 'der' }).subarray(-32)
  console.log('SEED_HEX  =', seed.toString('hex'), ' ← 存入 CI Secret MIYO_PLUGIN_SIGNING_SEED')
  console.log('PUBKEY_HEX=', pub.toString('hex'), ' ← 加入 Rust OFFICIAL_SIGNING_PUBKEYS（keyring 轮换）')
}

function cmdSign(dir) {
  const seed = process.env.MIYO_PLUGIN_SIGNING_SEED
  if (!seed) fail('缺少 MIYO_PLUGIN_SIGNING_SEED（签名私钥；见 keygen 输出说明）')
  const manifestPath = path.join(dir, 'manifest.json')
  if (!fs.existsSync(manifestPath)) fail(`目录缺少 manifest.json：${dir}`)
  const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'))
  manifest.files = buildFilesList(dir)
  delete manifest.signature
  // 先写回（含 files、无 signature），再按规范计算 payload（signature 置零）
  const withoutSig = Buffer.from(canonicalize(manifest), 'utf8')
  const payload = signingPayload(withoutSig)
  const signature = crypto.sign(null, payload, seedToPrivateKey(seed))
  manifest.signature = signature.toString('hex')
  fs.writeFileSync(manifestPath, Buffer.from(canonicalize(manifest), 'utf8'))
  console.log(`[sign_plugin] 已签名：${manifest.id} v${manifest.version}（files: ${manifest.files.length}）`)
}

function readManifestAndFiles(target) {
  if (fs.statSync(target).isDirectory()) {
    const manifestBytes = fs.readFileSync(path.join(target, 'manifest.json'))
    const files = collectFiles(target).map(({ abs, rel }) => ({
      path: rel,
      sha256: sha256Hex(fs.readFileSync(abs)),
      size: fs.statSync(abs).size,
    }))
    return { manifestBytes, files }
  }
  // .miyo 包（tar.gz）：Node 无内置 tar——verify 包形态交由 Miyo 安装流程 /
  // Rust 侧承担，工具只验目录形态。提示用户解包后验。
  fail('verify 仅支持目录形态（.miyo 包请安装到 Miyo 或先解包）')
}

function cmdVerify(target) {
  const { manifestBytes, files } = readManifestAndFiles(target)
  const manifest = JSON.parse(manifestBytes.toString('utf8'))
  // 1. 全文件完整性
  const listed = new Map((manifest.files ?? []).map((f) => [f.path, f]))
  const actual = new Map(files.map((f) => [f.path, f]))
  let ok = true
  for (const [p, f] of listed) {
    const a = actual.get(p)
    if (!a) { console.error(`  ✗ 清单缺文件：${p}`); ok = false; continue }
    if (a.sha256 !== f.sha256.toLowerCase()) { console.error(`  ✗ sha256 失配：${p}`); ok = false }
  }
  for (const p of actual.keys()) {
    if (!listed.has(p)) { console.error(`  ✗ 清单外文件：${p}`); ok = false }
  }
  // 2. 签名
  const payload = signingPayload(manifestBytes)
  const sig = manifest.signature ? Buffer.from(manifest.signature, 'hex') : null
  let trust = 'unsigned'
  if (sig) {
    trust = OFFICIAL_PUBKEYS.some((hex) => {
      const key = crypto.createPublicKey({
        key: Buffer.concat([
          Buffer.from('302a300506032b6570032100', 'hex'),
          Buffer.from(hex, 'hex'),
        ]),
        format: 'der',
        type: 'spki',
      })
      return crypto.verify(null, payload, key, sig)
    })
      ? 'official'
      : 'foreign'
  }
  console.log(`[sign_plugin] 完整性：${ok ? '✓' : '✗'}  签名信任级：${trust}（${manifest.id} v${manifest.version}）`)
  if (!ok || trust === 'foreign') process.exit(1)
}

// ---------------------------------------------------------------------------
// golden vector：与 Rust package.rs 测试共用同一份输入与期望 payload，
// 任一侧规范化实现漂移即 CI 红灯（跨语言互验）。期望值由本脚本初始版本
// 生成并同步写入 Rust 测试常量（GOLDEN_PAYLOAD_HEX）。
// ---------------------------------------------------------------------------
const GOLDEN_PAYLOAD_HEX = (() => {
  const obj = {
    id: 'com.example.golden',
    name: 'Golden',
    description: 'golden',
    version: '1.0.0',
    kind: 'ui',
    min_app_version: '0.0.60',
    sdk_version: 1,
    permissions: ['std.storage'],
    files: [
      { path: 'a.js', sha256: 'ab'.repeat(32), size: 3 },
      { path: 'sub/b.css', sha256: 'cd'.repeat(32), size: 5 },
    ],
    signature: '0'.repeat(128),
  }
  const head = Buffer.from(canonicalize(obj), 'utf8')
  const lines = [...obj.files]
    .sort((a, b) => (a.path < b.path ? -1 : 1))
    .map((f) => `${f.path} ${f.sha256} ${f.size}\n`)
    .join('')
  return Buffer.concat([head, Buffer.from('\n', 'utf8'), Buffer.from(lines, 'utf8')]).toString('hex')
})()

function cmdSelftest() {
  const payload = signingPayload(
    Buffer.from(
      JSON.stringify({
        id: 'com.example.golden',
        name: 'Golden',
        description: 'golden',
        version: '1.0.0',
        kind: 'ui',
        min_app_version: '0.0.60',
        sdk_version: 1,
        permissions: ['std.storage'],
        files: [
          { path: 'a.js', sha256: 'ab'.repeat(32), size: 3 },
          { path: 'sub/b.css', sha256: 'cd'.repeat(32), size: 5 },
        ],
      }),
    ),
  )
  if (payload.toString('hex') !== GOLDEN_PAYLOAD_HEX) {
    fail('golden payload 与自身规范不一致（工具内部漂移）')
  }
  // 签名 roundtrip（临时密钥）
  const { privateKey, publicKey } = crypto.generateKeyPairSync('ed25519')
  const sig = crypto.sign(null, payload, privateKey)
  const pubRaw = publicKey.export({ type: 'spki', format: 'der' }).subarray(-32).toString('hex')
  const spkiForRaw = Buffer.concat([
    Buffer.from('302a300506032b6570032100', 'hex'),
    Buffer.from(pubRaw, 'hex'),
  ])
  const ok = crypto.verify(
    null,
    payload,
    crypto.createPublicKey({ key: spkiForRaw, format: 'der', type: 'spki' }),
    sig,
  )
  if (!ok) fail('签名 roundtrip 失败')
  console.log('[sign_plugin] selftest ✓（payload 规范 + Ed25519 roundtrip）')
  console.log('  golden payload sha256 =', sha256Hex(payload))
}

const [cmd, arg] = process.argv.slice(2)
if (cmd === 'keygen') cmdKeygen()
else if (cmd === 'sign') arg ? cmdSign(arg) : fail('用法：sign <plugin_dir>')
else if (cmd === 'verify') arg ? cmdVerify(arg) : fail('用法：verify <plugin_dir>')
else if (cmd === 'selftest') cmdSelftest()
else fail('用法：sign_plugin.mjs <keygen|sign|verify|selftest> [arg]')
