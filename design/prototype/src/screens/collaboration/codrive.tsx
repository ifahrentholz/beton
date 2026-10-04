import { Globe as Browser, Monitor, Smartphone } from 'lucide-react'
import { Composer } from '@/app/session-chrome'
import { AgentMessage, ApprovalCard, SystemNote, ToolCall, UserMessage } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { Avatar, Menu, MenuLabel, SessionShell } from '@/app/kit/workspace'

function PresenceStack({ open }: { open?: boolean }) {
  return (
    <F id="COL-004" as="span" className="relative inline-flex items-center" badge="bottom-left">
      <span className="flex -space-x-1.5" aria-label="Gerade in dieser Session: Ingo, Anna, Jonas">
        <Avatar name="Ingo Fahrentholz" size="sm" ring title="Du · Desktop" />
        <Avatar name="Anna Becker" size="sm" ring title="Anna Becker · Browser · src/routes/auth.ts" />
        <Avatar name="Jonas Weber" size="sm" muted title="Jonas Weber · Handy · nicht im Vordergrund" />
      </span>
      {open && (
        <span className="absolute top-8 right-0 z-30">
          <Menu className="w-72">
            <MenuLabel>Gerade in dieser Session</MenuLabel>
            {[
              { n: 'Ingo Fahrentholz', who: 'Du', icon: Monitor, where: 'Desktop · Verlauf', focused: true },
              { n: 'Anna Becker', who: 'Anna Becker', icon: Browser, where: 'Browser · Datei src/routes/auth.ts', focused: true },
              { n: 'Jonas Weber', who: 'Jonas Weber', icon: Smartphone, where: 'Handy · App im Hintergrund', focused: false },
            ].map((p) => (
              <div key={p.n} className="mx-1 flex items-center gap-2 px-2 py-1.5 text-[13px]">
                <Avatar name={p.n} size="sm" ring={p.focused} muted={!p.focused} />
                <span className="min-w-0 flex-1">
                  <span className="block">{p.who}</span>
                  <span className="flex items-center gap-1 text-[11px] text-muted-foreground">
                    <p.icon className="size-3" /> {p.where}
                  </span>
                </span>
                <span className="text-[11px] text-muted-foreground">{p.focused ? 'aktiv' : 'abwesend'}</span>
              </div>
            ))}
          </Menu>
        </span>
      )}
    </F>
  )
}

export function CoDriveScreen({ state }: { state: string }) {
  return (
    <SessionShell
      connection="server"
      status="running"
      headerExtra={<PresenceStack open={state === 'presence'} />}
      composer={
        <div>
          {state === 'typing' && (
            <F id="COL-004" className="flex items-center gap-2 px-4 pt-2 text-[12px] text-muted-foreground">
              <Avatar name="Anna Becker" size="sm" />
              Anna tippt …
            </F>
          )}
          <F id={['COL-003', 'SES-004']}>
            <Composer
              harness="claude"
              running
              queued={['Anna Becker: Danach bitte die README um die Limits ergänzen', 'Du: Und den Wert 5 aus der Config lesen, nicht hart kodieren']}
            />
          </F>
        </div>
      }
    >
      <UserMessage author="Ingo">Die Login-Route braucht einen Rate-Limiter: höchstens 5 Versuche pro Minute und IP. Bitte mit Tests.</UserMessage>
      <ToolCall kind="edit" name="Bearbeiten" target="src/middleware/rate-limit.ts" duration="0,3 s" />
      <AgentMessage harness="claude">
        <p>Der Limiter steht und ist an der Login-Route eingehängt. 10 Tests laufen.</p>
      </AgentMessage>
      <F id="COL-003" className="flex flex-col gap-4">
        <SystemNote>Anna Becker steuert mit · ab jetzt sieht der Agent, wer schreibt</SystemNote>
        <div>
          <UserMessage author="Anna">Wenn jemand eingeloggt ist, bitte nach User limitieren statt nach IP. Sonst sperren wir ganze Büros hinter einem NAT aus.</UserMessage>
          <div className="mt-1 ml-9 font-mono text-[11px] text-muted-foreground" title="So kommt die Nachricht beim Harness an">
            Im Modell-Kontext: [Anna Becker (@anna)]: Wenn jemand eingeloggt ist, …
          </div>
        </div>
        <AgentMessage harness="claude" streaming={state !== 'approval-decided'}>
          <p>Anna, guter Punkt. Ich nehme die User-ID, wenn vorhanden, sonst die IP, und ergänze einen Test für zwei Nutzer hinter derselben IP.</p>
        </AgentMessage>
      </F>
      {state === 'approval-decided' && (
        <>
          <ToolCall kind="shell" name="Shell" target="pnpm vitest run middleware" duration="3,9 s" policy="tests-fragen" />
          <F id="COL-003">
            <ApprovalCard tool="Shell" command="pnpm vitest run middleware" reason="" resolved="allowed" />
            <div className="mt-1 ml-9 flex items-center gap-1.5 text-[12px] text-muted-foreground">
              <Avatar name="Anna Becker" size="sm" /> Entschieden von Anna Becker · 14:12 · deine Inbox-Meldung ist erledigt
            </div>
          </F>
        </>
      )}
    </SessionShell>
  )
}
