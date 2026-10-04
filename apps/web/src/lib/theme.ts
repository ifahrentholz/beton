/** Hell/dunkel nach Systemeinstellung (Token-Sätze aus dem Design, `.dark`). */
export function followSystemTheme(): void {
  const mq = window.matchMedia('(prefers-color-scheme: dark)')
  const apply = () => document.documentElement.classList.toggle('dark', mq.matches)
  apply()
  mq.addEventListener('change', apply)
}
