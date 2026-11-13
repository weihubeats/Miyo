import { describe, expect, it } from 'vitest'
import {
  planCrossGroupMove,
  planSameGroupMove,
  wouldCreateCycle,
  type Orderable,
} from '@/stores/treeOps'

function group(...orders: number[]): Orderable[] {
  return orders.map((o, i) => ({ id: `i${i}`, sort_order: o }))
}

describe('planSameGroupMove', () => {
  it('组内下移：目标位之后顺延 +1，被移项落位目标位', () => {
    // [A(0), B(1), C(2)] 把 A 移到 2 → [B(0), C(1), A(2)]
    const g = group(0, 1, 2)
    const changed = planSameGroupMove(g, 'i0', 2)
    expect(changed.get('i0')).toBe(2)
    expect(changed.get('i1')).toBe(0)
    expect(changed.get('i2')).toBe(1)
  })

  it('组内上移：目标位之前的保持原序', () => {
    // [A(0), B(1), C(2)] 把 C 移到 0 → [C(0), A(1), B(2)]
    const g = group(0, 1, 2)
    const changed = planSameGroupMove(g, 'i2', 0)
    expect(changed.get('i2')).toBe(0)
    expect(changed.get('i0')).toBe(1)
    expect(changed.get('i1')).toBe(2)
  })

  it('目标位越界钳制到组尾', () => {
    const g = group(0, 1, 2)
    const changed = planSameGroupMove(g, 'i1', 99)
    expect(changed.get('i1')).toBe(2)
    expect(changed.get('i2')).toBe(1)
  })

  it('顺序未变的项不出现在结果里（避免多余落库）', () => {
    // [A(0), B(1), C(2)] 把 C 移到 2（原位）：无人变化
    const g = group(0, 1, 2)
    const changed = planSameGroupMove(g, 'i2', 2)
    expect(changed.get('i1')).toBeUndefined()
    expect(changed.get('i0')).toBeUndefined()
  })
})

describe('planCrossGroupMove', () => {
  it('跨组：新组按插入位重排，被移项落位新组目标位', () => {
    const oldG: Orderable[] = [
      { id: 'o0', sort_order: 0 },
      { id: 'o1', sort_order: 1 },
    ]
    const newG: Orderable[] = [
      { id: 'n0', sort_order: 0 },
      { id: 'n1', sort_order: 1 },
    ]
    // o0 移到新组下标 1：n0 保持 0，n1 顺延 2，o0 落位 1；o1 顺序未变不落库
    const changed = planCrossGroupMove(oldG, newG, 'o0', 1)
    expect(changed.get('o0')).toBe(1)
    expect(changed.get('n1')).toBe(2)
    expect(changed.get('o1')).toBeUndefined()
    expect(changed.get('n0')).toBeUndefined()
  })
})

describe('wouldCreateCycle', () => {
  const folders = [
    { id: 'a', parent_id: null },
    { id: 'b', parent_id: 'a' },
    { id: 'c', parent_id: 'b' },
  ]
  it('移入自身或子孙链判定为成环', () => {
    expect(wouldCreateCycle(folders, 'a', 'a')).toBe(true)
    expect(wouldCreateCycle(folders, 'a', 'b')).toBe(true)
    expect(wouldCreateCycle(folders, 'a', 'c')).toBe(true)
  })
  it('移入根或无关节点不成环', () => {
    expect(wouldCreateCycle(folders, 'a', null)).toBe(false)
    expect(wouldCreateCycle(folders, 'c', 'a')).toBe(false)
  })
})
