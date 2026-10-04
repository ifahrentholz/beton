import { cleanup, render } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import { Markdown } from '@/components/markdown'

afterEach(() => cleanup())

function blocks(text: string, streaming: boolean) {
  const { container } = render(<Markdown text={text} streaming={streaming} />)
  const md = container.querySelector('.md')!
  return [...md.children].map((el) => el.tagName)
}

describe('Markdown', () => {
  it('WEB-002 AC1: unvollständiger Codeblock rendert schon als Block (kein Sprung beim Schließen)', () => {
    const open = blocks('Text davor\n\n```rust\nfn main() {\n    println!("x");', true)
    const closed = blocks('Text davor\n\n```rust\nfn main() {\n    println!("x");\n}\n```', true)
    expect(open).toEqual(['P', 'PRE'])
    expect(closed).toEqual(open)
  })

  it('rendert kein Roh-HTML aus dem Modell', () => {
    const { container } = render(<Markdown text={'<img src=x onerror="alert(1)">**fett**'} />)
    expect(container.querySelector('img')).toBeNull()
    expect(container.querySelector('strong')?.textContent).toBe('fett')
  })
})

describe('Blöcke für inkrementelles Rendering', () => {
  it('teilt an Leerzeilen, nie innerhalb von Code-Fences', async () => {
    const { splitBlocks } = await import('@/components/markdown')
    expect(splitBlocks('a\n\nb')).toEqual(['a', 'b'])
    expect(splitBlocks('x\n\n```\n1\n\n2\n```\n\ny')).toEqual(['x', '```\n1\n\n2\n```', 'y'])
    expect(splitBlocks('- a\n\n- b')).toEqual(['- a\n\n- b'])
    expect(splitBlocks('')).toEqual([''])
  })
})
