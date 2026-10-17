// Aufrufe an die Desktop-App. Nur diese lokalen Seiten dürfen sie nutzen; Seiten des Daemons
// haben keinen Zugriff (Capability `local-pages`).
export function invoke(cmd, args) {
  return window.__TAURI__.core.invoke(cmd, args)
}

export function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag)
  for (const [k, v] of Object.entries(attrs)) {
    if (k === 'class') node.className = v
    else node.setAttribute(k, v)
  }
  for (const c of children) node.append(c)
  return node
}
