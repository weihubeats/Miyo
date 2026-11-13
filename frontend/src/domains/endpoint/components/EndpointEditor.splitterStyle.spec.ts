/**
 * EndpointEditor 样式契约：请求面板底部保持「唯一一条分割线」（Single Border Architecture）。
 *
 * 背景：请求卡（.config-box）与响应卡（.rp）各带完整边框时，接缝处会出现
 * 平行双线，用户难以判断哪条是真正的上下拖拽把手。两卡已并入 .req-resp-stack，
 * 分割条即唯一那条线；低频的请求设置（超时/重定向）收进页签行 Popover，
 * 接缝处不插入任何常驻工具行（用户实测：夹在 Body 与响应之间显乱）。
 *
 * jsdom 不做样式计算（getComputedStyle 拿不到 scoped 规则），故按源码断言——
 * 与本仓 themePattern.spec.ts 的守卫方式一致：直接读 .vue 文本查契约。
 */
import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

const SOURCE = readFileSync(
  join(process.cwd(), 'src/domains/endpoint/components/EndpointEditor.vue'),
  'utf8',
)

/** 取某个 CSS 选择器的规则体（从头一个 `{` 配平到 `}`），供精确断言。
 * 必须在行首匹配选择器，否则 `.config-box` 会先命中
 * `.req-resp-stack > .config-box` 这类复合选择器（子串包含），断言打在错的规则上。
 * 支持多行选择器（逗号分组），空白按正则 `\s+` 归一。 */
function ruleBody(selector: string): string {
  const escaped = selector
    .split(/\s+/)
    .map((part) => part.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))
    .join('\\s+')
  const re = new RegExp(`^${escaped} \\{`, 'm')
  const match = re.exec(SOURCE)
  expect(match, `未找到规则 ${selector}`).not.toBeNull()
  const open = SOURCE.indexOf('{', match!.index)
  let depth = 0
  for (let i = open; i < SOURCE.length; i += 1) {
    if (SOURCE[i] === '{') depth += 1
    else if (SOURCE[i] === '}') {
      depth -= 1
      if (depth === 0) return SOURCE.slice(open + 1, i)
    }
  }
  throw new Error(`规则 ${selector} 花括号未配平`)
}

describe('EndpointEditor 底部单分割线契约', () => {
  it('设置行已收进页签行 Popover：无常驻设置行、触发钮在请求卡内', () => {
    expect(SOURCE, '常驻设置行不得回归（夹在 Body 与响应之间显乱）').not.toContain(
      'class="req-settings"',
    )
    expect(SOURCE, '页签行须有请求设置触发钮').toContain('rs-trigger rs-settings')
    // config-box 是 overflow 滚动容器，内部绝对定位会被裁剪 → 必须 Teleport
    expect(SOURCE, '设置面板须 Teleport 到 body').toContain('<Teleport to="body">')
  })

  it('分割条自身就是那条接缝线（1px，位于 Body 编辑器正下方）', () => {
    const body = ruleBody('.rp-splitter')
    // 线由分割条本人画：高 1px + 主题色背景，线与把手是同一像素行
    expect(body, '分割条高度须为 1px（等于线本身，不额外占位）').toMatch(/height:\s*1px/)
    expect(body, '静止时须画出那条线').toMatch(/background:\s*var\(--border-panel\)/)
    expect(body, '不得用负 margin 错位').not.toMatch(/margin:\s*-/)
  })

  it('拖拽热区完全透明且以线为中心外扩（有背景就成第二条线）', () => {
    const body = ruleBody('.rp-splitter::after')
    expect(body, '::after 是命中热区，必须透明').not.toMatch(/background\s*:/)
    expect(body, '::after 需上下外扩命中范围').toMatch(/top:\s*-\d+px/)
    expect(body).toMatch(/bottom:\s*-\d+px/)
  })

  it('可拖提示线默认不可见，仅 hover / 拖拽中浮现，且不改盒高（避免拖动前跳动）', () => {
    const before = ruleBody('.rp-splitter::before')
    expect(before, '默认高度必须为 0（静止时不能有第二条线）').toMatch(/height:\s*0\b/)
    expect(before, '默认不可见').toMatch(/opacity:\s*0\b/)
    expect(before, '提示线走主题变量，不得写死色值').toContain('var(--accent)')
    expect(before, '必须是绝对定位的叠加层，不参与布局').toContain('position: absolute')
    // 分割条高 1px = 线本身，故提示线以自身中心对齐即可与真实线重合
    expect(before, '提示线须对齐真实分割线（居中）').toMatch(/top:\s*50%/)
    expect(before, '居中需补偿自身高度').toMatch(/transform:\s*translateY\(-50%\)/)

    // hover / dragging 才显形
    expect(SOURCE).toMatch(/\.rp-splitter:hover::before[\s\S]{0,240}?height:\s*[1-9]/)
    expect(SOURCE).toMatch(/\.rp-splitter\.dragging::before/)
    // dragging 类确实被绑定（否则上面的规则是死代码）
    expect(SOURCE).toContain(':class="{ dragging: splitterDragging }"')
  })
})

/**
 * 「两条线 + 拖不动」回归：这是用户实测报上来的问题，单独锁住。
 *
 * 成因：请求卡（.config-box）与响应卡（.rp）各带完整边框，又相距 .editor 的
 * 14px gap → 两条平行线；而分割条被负 margin 提到两卡之间，热区与两条线都不
 * 重合 → 「看着有线，拖上去没反应」。
 *
 * 修法：两卡并入 .req-resp-stack，接缝处只留一条线，分割条正落在接缝上。
 */
describe('EndpointEditor 两卡接缝：只允许一条线且热区落在线上', () => {
  it('请求卡与响应卡被同一个容器收拢（否则必然出现两条平行线）', () => {
    expect(SOURCE, '模板需有合并容器').toMatch(/class="req-resp-stack"/)
    // 容器内两卡之间不得再有 editor 的 14px gap 起作用：靠 flex 紧贴
    const stack = ruleBody('.req-resp-stack')
    expect(stack, '容器须为纵向 flex').toMatch(/flex-direction:\s*column/)
    // 容器才是那个「唯一外框」的持有者
    expect(stack, '外框由容器统一提供').toMatch(/border:\s*1px solid/)
  })

  it('请求卡自身不再画外框（其底边框就是多余的第二条线）', () => {
    const override = ruleBody('.req-resp-stack > .config-box')
    expect(override, '请求卡在容器内须去掉自身边框').toMatch(/border:\s*none/)
    expect(override, '在容器内不得有圆角（圆角由容器统一提供）').toMatch(/border-radius:\s*0/)
    expect(override, '不得再有自身阴影').toMatch(/box-shadow:\s*none/)
  })

  it('接缝处只有一条线：分割条画线，其余各层都不得再画', () => {
    // 分割条是线的主人
    expect(ruleBody('.rp-splitter'), '分割条须画出那条线').toMatch(
      /background:\s*var\(--border-panel\)/,
    )
    // 响应侧不再画线
    const resp = ruleBody(
      '.req-resp-stack .response-zone,\n.req-resp-stack .response-hint',
    )
    expect(resp, '响应侧不得重复画线').toMatch(/border-top:\s*none/)
    // 请求卡不得有边框
    expect(
      ruleBody('.req-resp-stack > .config-box'),
      '请求卡不得画边框',
    ).toMatch(/border:\s*none/)
  })

  it('分割条之下紧邻响应区（接缝处不得再插入常驻工具行）', () => {
    const splitterAt = SOURCE.indexOf('class="rp-splitter"')
    const respAt = SOURCE.indexOf('<template v-if="hasResponse">')
    expect(splitterAt, '未找到分割条').toBeGreaterThan(-1)
    expect(respAt, '未找到响应区').toBeGreaterThan(-1)
    expect(respAt, '响应区须排在分割条之后').toBeGreaterThan(splitterAt)
    const between = SOURCE.slice(splitterAt, respAt)
    expect(between, '接缝处不得插入常驻行（设置行已收进 Popover）').not.toContain('class="rs-')
  })

  it('分割条不再靠负 margin 错位（这是「热区与视觉线不重合」的根因）', () => {
    const body = ruleBody('.rp-splitter')
    expect(body, '不得再用负 margin 提位置，否则实际盒与视觉边界错位').not.toMatch(
      /margin:\s*-/,
    )
    expect(body, '接缝处无 gap，margin 应为 0').toMatch(/margin:\s*0\b/)
  })

  it('命中热区足够大且以接缝为中心（否则起点略偏就抓不住）', () => {
    const hit = ruleBody('.rp-splitter::after')
    expect(hit, '热区须上下外扩').toMatch(/top:\s*-\d+px/)
    expect(hit).toMatch(/bottom:\s*-\d+px/)
    const top = Number(/top:\s*-(\d+)px/.exec(hit)![1])
    const bottom = Number(/bottom:\s*-(\d+)px/.exec(hit)![1])
    const box = Number(/height:\s*(\d+)px/.exec(ruleBody('.rp-splitter'))![1])
    // 总热区高度须远大于 7px 盒高，才能「不用瞄」
    expect(top + box + bottom, '热区总高应 ≥ 20px，保证不用精准瞄准').toBeGreaterThanOrEqual(20)
  })

  it('鼠标点击不留「选中框」（仅键盘聚焦才出现描边）', () => {
    expect(SOURCE, '需显式抹掉鼠标聚焦下的描边').toMatch(
      /\.rp-splitter:focus:not\(:focus-visible\)[\s\S]{0,80}?outline:\s*none/,
    )
    expect(SOURCE, '键盘聚焦仍须可见').toMatch(/\.rp-splitter:focus-visible[\s\S]{0,120}?outline:/)
  })
})

/**
 * 「stylescope 届不到 + cascade 顺序」回归：两条线复发的真正原因。
 *
 * 第一次修完仍复发，是因为踩了两个互相独立的坑：
 * 1. `.rp` 在 ResponsePanel.vue 内部，带的是**它自己的** scoped 属性。本组件写
 *    `.req-resp-stack .rp` 会被编译成 `... .rp[data-v-本组件]`——元素上根本没有
 *    这个属性，规则**永不匹配**，响应卡自带的完整边框原样保留。
 *    必须用 `:deep()` 把属性选择器挂到祖先上才届得到。
 * 2. 覆盖块即使写对了，若排在基础规则**之前**，同权重下仍被后者反超。
 *    本组件样式表里 `.config-box` / `.response-hint` / `.response-zone` 的基础
 *    规则分散在中后段，覆盖块必须落在**文件末尾**。
 */
describe('EndpointEditor 接缝覆盖：子组件作用域穿透 + 级联顺序', () => {
  it('子组件内部的类用 :deep() 穿透（否则规则永远匹配不上）', () => {
    expect(SOURCE, '合并容器须用 :deep() 穿透到 ResponsePanel 的 .rp').toMatch(
      /\.req-resp-stack\s+:deep\(\.rp\)/,
    )
    // 反向守卫：不得再写「不穿透」的裸后代选择器，那是永不生效的死规则
    expect(SOURCE, '不得再写不穿透的 .req-resp-stack .rp').not.toMatch(
      /\.req-resp-stack\s+\.rp\s*[,{]/,
    )
  })

  it('覆盖块位于样式表末尾（排在所有基础规则之后，否则被反超）', () => {
    const overrideAt = SOURCE.indexOf('.req-resp-stack > .config-box')
    expect(overrideAt, '未找到合并容器的覆盖块').toBeGreaterThan(-1)

    // 每个可能反超的基础规则都必须出现在覆盖块之前
    for (const base of ['\n.config-box {', '\n.response-hint {', '\n.response-zone {']) {
      const baseAt = SOURCE.indexOf(base)
      expect(baseAt, `未找到基础规则 ${base.trim()}`).toBeGreaterThan(-1)
      expect(
        baseAt,
        `基础规则 ${base.trim()} 出现在覆盖块之后，会把接缝覆盖掉 → 两条线复发`,
      ).toBeLessThan(overrideAt)
    }

    const styleClose = SOURCE.lastIndexOf('</style>')
    expect(overrideAt, '覆盖块须在本文件样式表之内').toBeLessThan(styleClose)
  })

  it('样式表结构完整：<style> 与 </style> 成对且不提前闭合', () => {
    const opens = (SOURCE.match(/<style\b/g) ?? []).length
    const closes = (SOURCE.match(/<\/style>/g) ?? []).length
    expect(opens, '<style> 与 </style> 必须成对').toBe(closes)
    // 提前闭合会把后面的规则变成死代码（本轮曾出现，且不报错、只静默失效）
    const firstClose = SOURCE.indexOf('</style>')
    const lastClose = SOURCE.lastIndexOf('</style>')
    expect(firstClose, '不得出现提前闭合的 </style>（其后的 CSS 会静默失效）').toBe(lastClose)
  })
})