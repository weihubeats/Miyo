/**
 * pasteFormat 单测：共用引擎的位置规则（整篇替换 / 只动粘贴块 / 已美化放行）
 * 与「外部已排版文本」的落地规则（placeFormattedText）。
 */
import { describe, expect, it } from 'vitest'
import { autoFormatPasted, placeFormattedText } from '@/utils/pasteFormat'

const opts = {
  format: (src: string) => src.replace(/, */g, ', '),
  looksLike: (src: string) => src.startsWith('{'),
}

describe('autoFormatPasted', () => {
  it('空文档：排版整篇并落光标到末尾', () => {
    expect(autoFormatPasted('', 0, 0, '{a,b}', opts)).toEqual({ text: '{a, b}', caret: 6 })
  })

  it('文档已有内容：只排版粘贴块，光标落在粘贴块末尾', () => {
    const out = autoFormatPasted('xx{}yy', 2, 4, '{a,b}', opts)
    expect(out).toEqual({ text: 'xx{a, b}yy', caret: 8 })
  })

  it('已排版内容不拦截；looksLike 不认的不拦截；超长不拦截', () => {
    expect(autoFormatPasted('', 0, 0, '{a, b}', opts)).toBeNull()
    expect(autoFormatPasted('', 0, 0, 'plain', opts)).toBeNull()
    expect(autoFormatPasted('', 0, 0, `{${'x'.repeat(200_001)}}`, opts)).toBeNull()
  })

  it('排版器抛错当作「不排版」', () => {
    const boom = {
      format: () => {
        throw new Error('bad')
      },
      looksLike: () => true,
    }
    expect(autoFormatPasted('', 0, 0, '{a}', boom)).toBeNull()
  })
})

describe('placeFormattedText', () => {
  it('空文档 / 全选：整篇替换', () => {
    expect(placeFormattedText('old', 0, 3, 'new')).toEqual({ text: 'new', caret: 3 })
    expect(placeFormattedText('  ', 0, 0, 'new')).toEqual({ text: 'new', caret: 3 })
  })

  it('光标处插入并尊重选区', () => {
    expect(placeFormattedText('ab', 1, 1, 'XY')).toEqual({ text: 'aXYb', caret: 3 })
    expect(placeFormattedText('ab', 0, 2, 'XY')).toEqual({ text: 'XY', caret: 2 })
  })
})
