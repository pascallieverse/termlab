import { Button } from '@/components/el/button';
import { cn } from '@/lib/utils';
import type { ApprovalChoice } from '@/types';

/** Allow once / Always allow / Deny, as Termlab shows them. */
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
            <p
                role="status"
                className={cn('m-0 text-[13px] text-dim', className)}
            >
                {pending}
            </p>
        );
    }

    const pad = size === 'sm' ? 'px-3 rounded-[7px]' : 'px-3.5';

    if (layout === 'stack') {
        return (
            <div className={cn('flex flex-col gap-2', className)}>
                <Button
                    className="min-h-12 rounded-[10px]"
                    onClick={() => onAnswer('allow_once')}
                >
                    Allow once
                </Button>
                <div className="grid grid-cols-2 gap-2">
                    <Button
                        variant="outline"
                        className="min-h-[46px] rounded-[10px]"
                        onClick={() => onAnswer('allow_always')}
                    >
                        Always allow
                    </Button>
                    <Button
                        variant="danger"
                        className="min-h-[46px] rounded-[10px]"
                        onClick={() => onAnswer('deny')}
                    >
                        Deny
                    </Button>
                </div>
            </div>
        );
    }

    return (
        <div className={cn('flex flex-wrap gap-1.5', className)}>
            <Button className={pad} onClick={() => onAnswer('allow_once')}>
                Allow once
            </Button>
            <Button
                variant="outline"
                className={pad}
                onClick={() => onAnswer('allow_always')}
            >
                Always allow
            </Button>
            <Button
                variant="danger"
                className={pad}
                onClick={() => onAnswer('deny')}
            >
                Deny
            </Button>
        </div>
    );
}
