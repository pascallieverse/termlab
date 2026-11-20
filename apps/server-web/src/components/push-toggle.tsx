import { Button } from '@/components/el/button';
import { usePush } from '@/hooks/use-push';
import { cn } from '@/lib/utils';

/** "Notify me on this device": push notifications on or off for this browser or phone. */
export function PushToggle({
    compact = false,
    className,
    onChange,
}: {
    compact?: boolean;
    className?: string;
    onChange?: () => void;
}) {
    const push = usePush();

    const note = {
        loading: null,
        insecure: 'Notifications need a secure connection. Open this server over https:// to turn them on.',
        unsupported: 'This browser can’t show notifications. Use Chrome, Edge, Firefox or Safari.',
        install:
            'On iPhone and iPad, tap Share › Add to Home Screen first, then open Termlab Server from your Home Screen and turn notifications on there.',
        denied: 'Notifications are blocked for this site. Allow them in your browser’s site settings, then try again.',
        off: compact ? null : 'Get a notification when a session needs you or fails.',
        on: 'This device gets a notification when a session needs you or fails.',
    }[push.state];

    return (
        <div className={cn('flex flex-col gap-2', className)}>
            {(push.state === 'off' || push.state === 'on' || push.state === 'loading') && (
                <Button
                    variant={push.state === 'on' ? 'outline' : 'primary'}
                    disabled={push.state === 'loading'}
                    aria-pressed={push.state === 'on'}
                    onClick={async () => {
                        await (push.state === 'on' ? push.disable() : push.enable());
                        onChange?.();
                    }}
                    className={compact ? 'self-center' : 'self-start'}
                >
                    {push.state === 'on' ? 'Turn off notifications on this device' : 'Notify me on this device'}
                </Button>
            )}
            {note && <p className={cn('m-0 text-[13px] text-dim', compact && 'text-center')}>{note}</p>}
            {push.error && (
                <p role="alert" className="m-0 text-[13px] text-bad">
                    {push.error}
                </p>
            )}
        </div>
    );
}
