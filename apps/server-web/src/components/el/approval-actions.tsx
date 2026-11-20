import { Button } from '@/components/el/button';
import { cn } from '@/lib/utils';
import type { ApprovalChoice } from '@/types';

/** Allow once / Always allow in session / Deny, as the desktop app's approval bar shows them. */
export function ApprovalActions({
    onAnswer,
    pending,
    size = 'md',
    layout = 'row',
    className,
}: {
    onAnswer: (choice: ApprovalChoice) => void;
    pending?: string;
    size?: 'sm' | 'md';
    layout?: 'row' | 'stack';
    className?: string;
}) {
    if (pending) {
        return (
            <p role="status" className={cn('m-0 text-[13px] text-dim', className)}>
                {pending}
            </p>
        );
    }

    if (layout === 'stack') {
        return (
            <div className={cn('flex flex-col gap-2', className)}>
                <Button variant="primary" size="lg" onClick={() => onAnswer('allow_once')}>
                    Allow once
                </Button>
                <div className="grid grid-cols-2 gap-2">
                    <Button variant="amber" size="lg" onClick={() => onAnswer('allow_always')}>
                        Always allow
                    </Button>
                    <Button variant="outline" size="lg" onClick={() => onAnswer('deny')}>
                        Deny
                    </Button>
                </div>
            </div>
        );
    }

    return (
        <div className={cn('approval-actions', className)}>
            <Button variant="primary" size={size} onClick={() => onAnswer('allow_once')}>
                Allow once
            </Button>
            <Button variant="amber" size={size} onClick={() => onAnswer('allow_always')}>
                Always allow in session
            </Button>
            <Button variant="outline" size={size} onClick={() => onAnswer('deny')}>
                Deny
            </Button>
        </div>
    );
}
