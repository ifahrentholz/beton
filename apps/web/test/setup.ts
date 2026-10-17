// Bestehende Tests prüfen deutsche Texte: Sprache fest auf Deutsch (WEB-017). jsdom meldet sonst
// `en-US` als Browsersprache. Tests zur Sprachwahl setzen sie selbst.
if (typeof localStorage !== 'undefined') localStorage.setItem('beton.lang', 'de')
