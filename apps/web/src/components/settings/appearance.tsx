import { Check } from 'lucide-react'
import { detectLanguage, locale, useI18n, useT, type LangPref, type MessageKey } from '@/lib/i18n'
import { chordText } from '@/lib/keys'
import { command, shortcutOf, useKeymap } from '@/lib/commands'
import { setHighContrast, useTheme, type FontSize, type Motion, type ThemeId } from '@/lib/theme'
import { cn } from '@/lib/utils'
import { useUi } from '../ui-state'
import { Btn, PageHeader, Row, SectionTitle, Segmented, SettingsShell, Switch } from './kit'

const TILES: { id: ThemeId; name: MessageKey; note: MessageKey; theme: string; split?: boolean }[] = [
  { id: 'system', name: 'theme.system', note: 'theme.systemNote', theme: 'theme-concrete', split: true },
  { id: 'concrete-light', name: 'theme.concreteLight', note: 'theme.concreteLightNote', theme: 'theme-concrete light' },
  { id: 'concrete-dark', name: 'theme.concreteDark', note: 'theme.concreteDarkNote', theme: 'theme-concrete dark' },
  { id: 'nord', name: 'theme.nord', note: 'theme.nordNote', theme: 'theme-nord', split: true },
  { id: 'hc', name: 'theme.hc', note: 'theme.hcNote', theme: 'theme-hc', split: true },
]

/**
 * Vorschau eines Themes mit echten Tokens (UX-005): Familie und Modus stehen als Klassen am
 * Element, z. B. `theme-nord dark`. Mit `split` zeigt die Kachel links hell, rechts dunkel.
 */
function ThemeTile({ name, note, selected, theme, split, onSelect }: { name: string; note: string; selected: boolean; theme: string; split?: boolean | undefined; onSelect: () => void }) {
  const mini = (
    <div className="flex h-full bg-background">
      <div className="w-3 border-r border-border bg-sidebar" />
      <div className="flex flex-1 flex-col gap-1 p-1.5">
        <div className="h-1 w-3/5 rounded-full bg-foreground/70" />
        <div className="h-1 w-4/5 rounded-full bg-muted-foreground/50" />
        <div className="mt-auto flex gap-1">
          <span className="h-2 w-5 rounded-[2px] bg-voice-claude" />
          <span className="h-2 w-3 rounded-[2px] bg-voice-codex" />
          <span className="chamfer-sm ml-auto h-2 w-4 bg-signal" />
        </div>
      </div>
    </div>
  )
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      tabIndex={selected ? 0 : -1}
      onClick={onSelect}
      data-theme-tile
      className={cn('flex w-[112px] flex-col gap-1.5 rounded-md border p-1.5 text-left', selected ? 'border-foreground ring-1 ring-foreground' : 'border-border hover:bg-accent')}
    >
      <div aria-hidden className="relative h-16 overflow-hidden rounded-[3px] border border-border">
        {split ? (
          <div className="flex h-full">
            <div className={cn('w-1/2 overflow-hidden', theme, 'light')}>{mini}</div>
            <div className={cn('w-1/2 overflow-hidden', theme, 'dark')}>{mini}</div>
          </div>
        ) : (
          <div className={cn('h-full', theme)}>{mini}</div>
        )}
      </div>
      <div className="flex items-center gap-1 px-0.5 text-[12px] font-medium">
        {selected && <Check aria-hidden className="size-3" />}
        {name}
      </div>
      <div className="px-0.5 text-[11px] leading-tight text-muted-foreground">{note}</div>
    </button>
  )
}

/** Beispiel der Formate in der aktuellen Sprache (Datum, Uhrzeit, Zahl, Währung). */
export function formatSample(loc: string, now: Date): string {
  return [
    now.toLocaleDateString(loc, { day: '2-digit', month: '2-digit', year: 'numeric' }),
    now.toLocaleTimeString(loc, { hour: '2-digit', minute: '2-digit' }),
    (1234.5).toLocaleString(loc),
    (4.82).toLocaleString(loc, { style: 'currency', currency: 'USD' }),
  ].join(' · ')
}

/**
 * Darstellung (Entwurf `settings-appearance`): Theme (UX-005), Sprache (WEB-017) und
 * Barrierefreiheit (WEB-016). Alles gilt für dieses Gerät und wirkt sofort, ohne Neuladen.
 */
export function AppearanceSettingsView() {
  const t = useT()
  const appearance = useTheme()
  const { pref, lang, setPref } = useI18n()
  const overrides = useKeymap((s) => s.overrides)
  const loc = locale(lang)
  const detected = detectLanguage()
  const overlay = command('shortcuts.open')
  const selectTheme = (id: ThemeId) => appearance.update({ theme: id, beforeContrast: undefined })
  return (
    <SettingsShell active="/settings/appearance">
      <PageHeader title={t('appearance.title')}>{t('appearance.intro')}</PageHeader>

      <SectionTitle>{t('appearance.theme')}</SectionTitle>
      <div
        role="radiogroup"
        aria-label={t('appearance.theme')}
        className="flex flex-wrap gap-2"
        onKeyDown={(e) => {
          if (!['ArrowRight', 'ArrowLeft', 'ArrowDown', 'ArrowUp'].includes(e.key)) return
          e.preventDefault()
          const i = TILES.findIndex((x) => x.id === appearance.theme)
          const step = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : -1
          const next = TILES[(i + step + TILES.length) % TILES.length]!
          selectTheme(next.id)
          e.currentTarget.querySelectorAll<HTMLButtonElement>('[role=radio]')[TILES.indexOf(next)]?.focus()
        }}
      >
        {TILES.map((tile) => (
          <ThemeTile
            key={tile.id}
            name={t(tile.name)}
            note={t(tile.note)}
            theme={tile.theme}
            split={tile.split}
            selected={appearance.theme === tile.id}
            onSelect={() => selectTheme(tile.id)}
          />
        ))}
      </div>

      <div className="mt-2">
        <SectionTitle>{t('appearance.language')}</SectionTitle>
        <Row label={t('appearance.language')} hint={t('appearance.languageHint')}>
          <Segmented<LangPref>
            label={t('appearance.language')}
            value={pref}
            onChange={setPref}
            options={[
              { id: 'auto', label: `${t('appearance.auto')} (${t(detected === 'de' ? 'lang.de' : 'lang.en')})` },
              { id: 'de', label: 'Deutsch' },
              { id: 'en', label: 'English' },
            ]}
          />
        </Row>
        <Row label={t('appearance.formats')} hint={t('appearance.formatsHint')}>
          <span className="font-mono text-[12px] text-muted-foreground tabular-nums" data-testid="format-sample">
            {formatSample(loc, new Date())}
          </span>
        </Row>
      </div>

      <div className="mt-2">
        <SectionTitle>{t('appearance.a11y')}</SectionTitle>
        <Row label={t('appearance.motion')} hint={t('appearance.motionHint')}>
          <Segmented<Motion>
            label={t('appearance.motion')}
            value={appearance.motion}
            onChange={(motion) => appearance.update({ motion })}
            options={[
              { id: 'system', label: t('appearance.system') },
              { id: 'on', label: t('appearance.on') },
              { id: 'off', label: t('appearance.off') },
            ]}
          />
        </Row>
        <Row label={t('appearance.contrast')} hint={t('appearance.contrastHint')}>
          <Switch checked={appearance.theme === 'hc'} onChange={setHighContrast} label={t('appearance.contrast')} />
        </Row>
        <Row label={t('appearance.focus')} hint={t('appearance.focusHint')}>
          <Switch checked={appearance.focusAlways} onChange={(focusAlways) => appearance.update({ focusAlways })} label={t('appearance.focus')} />
        </Row>
        <Row label={t('appearance.live')} hint={t('appearance.liveHint')}>
          <Switch checked={appearance.announce} onChange={(announce) => appearance.update({ announce })} label={t('appearance.live')} />
        </Row>
        <Row label={t('appearance.size')}>
          <Segmented<`${FontSize}`>
            label={t('appearance.size')}
            value={`${appearance.fontSize}`}
            onChange={(v) => appearance.update({ fontSize: Number(v) as FontSize })}
            options={[
              { id: '13', label: '13 px' },
              { id: '14', label: '14 px' },
              { id: '16', label: '16 px' },
            ]}
          />
        </Row>
        <Row label={t('appearance.keyboard')} hint={t('appearance.keyboardHint')}>
          <Btn onClick={() => useUi.getState().set({ shortcuts: true, palette: false, switcher: false })}>
            {t('appearance.showKeys')} <kbd className="font-mono text-[11px] text-muted-foreground">{chordText((overlay && shortcutOf(overlay, overrides)) ?? 'Mod+/')}</kbd>
          </Btn>
        </Row>
      </div>
    </SettingsShell>
  )
}
