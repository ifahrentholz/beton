import { F } from '@/proto/feature-marker'
import { Cmd, Ln, Term } from '@/screens/harnesses/terminal'

export function AutomationCli({ state }: { state: string }) {
  if (state === 'queue')
    return (
      <Term>
        <F id={['ASY-012', 'ASY-011']}>
          <Cmd>beton schedule list --queue</Cmd>
          <Ln>Gleichzeitig: 3 von 4 (Host) · 3 von 8 (du)</Ln>
          <Ln />
          <Ln>Pos  Lauf     Agent     Auslöser            wartet  Grund</Ln>
          <Ln>1    run_99z  pr-fixer  webhook ci-failed   2 Min.  pr-fixer: 2 von 2 aktiv (async.max_concurrent_runs)</Ln>
        </F>
      </Term>
    )
  if (state === 'history')
    return (
      <Term>
        <F id="ASY-011">
          <Cmd>beton schedule run-now nightly-ci</Cmd>
          <Ln tone="ok">✓ Lauf run_98e gestartet (manuell, Einstellungen von nightly-ci)</Ln>
          <Cmd>beton run-history nightly-ci</Cmd>
          <Ln>Lauf     Status           Start             Dauer    Verbrauch          Session</Ln>
          <Ln>run_9a1  wartet (Freig.)  Sa. 03.10. 03:00  14 Min.  Claude Max · 3 %   ses_9a1c</Ln>
          <Ln>run_98e  fertig (manuell) Fr. 02.10. 14:22  21 Min.  Claude Max · 4 %   ses_98e2</Ln>
          <Ln tone="deny">run_95d  Zeitlimit        Fr. 02.10. 03:00  2 Std.   Claude Max · 11 %  ses_95d0</Ln>
          <Ln>run_93a  fertig           Do. 01.10. 03:00  3 Min.   Claude Max · 1 %   ses_93a7</Ln>
          <Ln tone="muted">–        übersprungen     Mi. 30.09. 03:00  –        –                  Vor-Lauf lief noch</Ln>
        </F>
      </Term>
    )
  return (
    <Term>
      <F id={['ASY-011', 'ASY-005', 'ASY-004']}>
        <Cmd>beton schedule list</Cmd>
        <Ln tone="deny">! Der beton-Dienst läuft nicht. Schedules und Timer werden nur ausgeführt, solange er läuft.</Ln>
        <Ln tone="muted">  Starten: beton daemon start · Mit der Anmeldung starten: beton daemon install-autostart</Ln>
        <Ln />
        <Ln>Name           Agent         Cron          Zeitzone       Nächster Lauf     Verpasst      Zustand</Ln>
        <Ln>nightly-ci     pr-fixer      0 3 * * *     Europe/Berlin  So. 04.10. 03:00  nachholen     aktiv</Ln>
        <Ln>weekly-deps    dep-updater   30 7 * * 6    Europe/Berlin  Sa. 10.10. 07:30  nachholen     aktiv</Ln>
        <Ln>weekly-report  pr-fixer      0 18 * * 5    Europe/Berlin  Fr. 09.10. 18:00  überspringen  aktiv</Ln>
        <Ln tone="muted">a11y-sweep     a11y-auditor  30 2 * * *    Europe/Berlin  –                 überspringen  pausiert</Ln>
      </F>
    </Term>
  )
}
