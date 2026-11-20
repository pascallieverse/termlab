import { useAppState } from '@/app-state';
import { LogoMark } from '@/components/el/logo';
import { Icon } from '@/layouts/nav';
import { useLive } from '@/live/LiveProvider';
import { timeAgo } from '@/lib/format';
import { Group, Row } from './Appearance';

const REPO = 'https://github.com/pascallieverse/termlab';

const LINKS = [
    { href: REPO, label: 'Source code', desc: 'github.com/pascallieverse/termlab' },
    { href: `${REPO}/blob/main/docs/self-hosting.md`, label: 'Self-hosting guide', desc: 'Install, HTTPS, backups and upgrades' },
    { href: `${REPO}/blob/main/docs/server-protocol.md`, label: 'API and protocol', desc: 'For scripts and integrations' },
    { href: `${REPO}/releases`, label: 'Releases', desc: 'New versions of Termlab and Termlab Server' },
];

/** Settings › About: the server's version, its license and source, and the linked Termlabs. */
export function AboutSection() {
    const { state } = useAppState();
    const live = useLive();
    const machines = [...live.machines].sort((a, b) => a.name.localeCompare(b.name));
    const name = state.server_name && state.server_name !== 'Termlab Server' ? state.server_name : null;

    return (
        <>
            <div className="about-hero">
                <span className="about-logo">
                    <LogoMark size={30} />
                </span>
                <div className="flex min-w-0 flex-col gap-0.5">
                    <div className="about-name">Termlab Server</div>
                    <div className="text-[13px] text-dim">
                        Version <span className="font-mono">{state.version}</span>
                        {name && <> · {name}</>}
                    </div>
                </div>
            </div>

            <Group
                title="Open source"
                note="Fair Source (FSL-1.1-ALv2): free to use, change and run yourself; each version becomes Apache 2.0 two years after its release."
            >
                {LINKS.map((l) => (
                    <a key={l.href} href={l.href} target="_blank" rel="noreferrer" className="settings-row clickable about-link">
                        <span className="settings-row-text">
                            <span className="settings-row-title">{l.label}</span>
                            <span className="text-xs text-dim">{l.desc}</span>
                        </span>
                        <span className="text-faint" aria-hidden="true">
                            <Icon name="external" size={14} />
                        </span>
                        <span className="sr-only">(opens in a new tab)</span>
                    </a>
                ))}
            </Group>

            <Group title="Linked machines" note="Each computer runs its own Termlab; update it from Termlab's Settings › About.">
                {machines.length === 0 ? (
                    <Row title="No machines yet" desc="Pair one under Machines." />
                ) : (
                    machines.map((m) => (
                        <Row
                            key={m.id}
                            title={
                                <>
                                    <span aria-hidden="true" className="dot" style={{ background: m.online ? '#3FB950' : '#8B929C' }} />
                                    {m.name}
                                    <span className="sr-only">{m.online ? '(online)' : '(offline)'}</span>
                                </>
                            }
                            desc={m.online ? (m.os ?? 'Online') : `Offline · seen ${timeAgo(m.last_seen_at)}`}
                        >
                            <span className="font-mono text-[13px] text-ink-soft">{m.version ? `Termlab ${m.version}` : '—'}</span>
                        </Row>
                    ))
                )}
            </Group>
        </>
    );
}
