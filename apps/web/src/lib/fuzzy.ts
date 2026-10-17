/**
 * Unscharfe Suche der Befehlspalette (UX-002). Jedes Suchwort muss im Titel oder in den
 * Suchwörtern vorkommen; Treffer am Wortanfang des Titels zählen am meisten, dann Treffer im
 * Titel, dann Suchwörter, zuletzt Buchstabenfolgen (`nsess` → „Neue Session“). 0 heißt: kein Treffer.
 */

const fold = (s: string) =>
  s
    .toLowerCase()
    .normalize('NFD')
    .replace(/[̀-ͯ]/g, '')
    .replace(/ß/g, 'ss')

function subsequence(needle: string, hay: string): boolean {
  let i = 0
  for (const ch of hay) if (ch === needle[i]) i += 1
  return i === needle.length
}

function scoreToken(token: string, title: string, words: string[], keywords: string[]): number {
  if (words.some((w) => w.startsWith(token))) return title.startsWith(token) ? 110 : 100
  if (title.includes(token)) return 60
  if (keywords.some((k) => k === token)) return 50
  if (keywords.some((k) => k.startsWith(token))) return 40
  if (keywords.some((k) => k.includes(token))) return 30
  if (subsequence(token, title.replace(/\s+/g, ''))) return 10
  return 0
}

export function fuzzyScore(query: string, title: string, keywords: readonly string[] = []): number {
  const tokens = fold(query).split(/\s+/).filter(Boolean)
  if (tokens.length === 0) return 1
  const t = fold(title)
  const words = t.split(/[^\p{L}\p{N}]+/u).filter(Boolean)
  const kws = keywords.map(fold)
  let total = 0
  for (const token of tokens) {
    const s = scoreToken(token, t, words, kws)
    if (s === 0) return 0
    total += s
  }
  return total
}
