import { create } from 'zustand'
import { de } from '@/i18n/de'
import { en } from '@/i18n/en'

/**
 * UI-Texte Deutsch/Englisch (WEB-017). Englisch ist Fallback; ohne Einstellung gilt die Sprache
 * von Browser bzw. Betriebssystem. Ein Wechsel wirkt sofort, ohne neu zu laden: Komponenten lesen
 * die Texte über `useT()` und rendern bei einem Wechsel neu. Bewusst ohne i18next: Zwei Sprachen
 * mit flachen Schlüsseln brauchen keine Bibliothek im initialen Bundle (WEB-015).
 */

export type Lang = 'de' | 'en'
export type LangPref = 'auto' | Lang
export type MessageKey = keyof typeof de
export type Messages = Record<MessageKey, string>
export type Vars = Record<string, string | number>
export type T = (key: MessageKey, vars?: Vars) => string

export const MESSAGES: Record<Lang, Messages> = { de, en }

const STORAGE_KEY = 'beton.lang'

/** Sprache aus Browser/OS: die erste unterstützte der bevorzugten Sprachen, sonst Englisch. */
export function detectLanguage(languages: readonly string[] = typeof navigator === 'undefined' ? [] : navigator.languages ?? [navigator.language]): Lang {
  for (const l of languages) {
    const base = l.toLowerCase().split('-')[0]
    if (base === 'de' || base === 'en') return base
  }
  return 'en'
}

function readPref(): LangPref {
  try {
    const v = localStorage.getItem(STORAGE_KEY)
    return v === 'de' || v === 'en' ? v : 'auto'
  } catch {
    return 'auto'
  }
}

/** Text zu einem Schlüssel; `{name}` wird ersetzt. Fehlt ein Text, gilt Englisch. */
export function translate(lang: Lang, key: MessageKey, vars?: Vars): string {
  const text = MESSAGES[lang][key] ?? MESSAGES.en[key] ?? key
  if (!vars) return text
  return text.replace(/\{(\w+)\}/g, (m, name: string) => (name in vars ? String(vars[name]) : m))
}

interface I18nState {
  pref: LangPref
  lang: Lang
  setPref: (pref: LangPref) => void
}

const resolve = (pref: LangPref): Lang => (pref === 'auto' ? detectLanguage() : pref)

export const useI18n = create<I18nState>()((set) => {
  const pref = readPref()
  return {
    pref,
    lang: resolve(pref),
    setPref: (next) => {
      try {
        if (next === 'auto') localStorage.removeItem(STORAGE_KEY)
        else localStorage.setItem(STORAGE_KEY, next)
      } catch {
        // Ohne Speicher gilt die Wahl bis zum Neuladen.
      }
      set({ pref: next, lang: resolve(next) })
    },
  }
})

// Sprache am Dokument (Screenreader, Silbentrennung).
if (typeof document !== 'undefined') {
  document.documentElement.lang = useI18n.getState().lang
  useI18n.subscribe((s) => {
    document.documentElement.lang = s.lang
  })
}

/** Übersetzer der aktuellen Sprache; Komponenten rendern bei einem Wechsel neu. */
export function useT(): T {
  const lang = useI18n((s) => s.lang)
  return (key, vars) => translate(lang, key, vars)
}

/** Übersetzer außerhalb von Komponenten (liest die aktuelle Sprache). */
export const t: T = (key, vars) => translate(useI18n.getState().lang, key, vars)

/** Locale für `Intl` (Datum, Zahlen, Währung folgen der Sprache). */
export function locale(lang: Lang = useI18n.getState().lang): string {
  return lang === 'de' ? 'de-DE' : 'en-US'
}

/** Titel als Übersetzungsschlüssel, z. B. für Befehle: `title: tk('cmd.palette')`. */
export const tk = (key: MessageKey) => (tr: T) => tr(key)
