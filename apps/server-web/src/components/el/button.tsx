import { Link } from 'react-router-dom';
import type { ComponentProps } from 'react';
import { cn } from '@/lib/utils';

export type ButtonVariant =
    | 'primary'
    | 'outline'
    | 'danger'
    | 'light'
    | 'ghost';
export type ButtonSize = 'sm' | 'md' | 'lg';

const variants: Record<ButtonVariant, string> = {
    primary:
        'bg-amber font-semibold text-amber-ink hover:bg-amber-soft hover:text-amber-ink',
    outline:
        'border border-line-strong bg-transparent text-ink hover:border-dim hover:bg-raised hover:text-ink',
    danger: 'border border-bad-line bg-transparent text-bad hover:bg-bad-bg hover:text-bad',
    light: 'bg-ink font-semibold text-bg hover:bg-white hover:text-bg',
    ghost: 'bg-transparent text-dim hover:bg-raised hover:text-ink',
};

const sizes: Record<ButtonSize, string> = {
    sm: 'min-h-9 rounded-lg px-3 text-[13px]',
    md: 'min-h-11 rounded-lg px-4 text-[15px]',
    lg: 'min-h-12 rounded-[10px] px-[22px] text-base',
};

export function buttonClass(
    variant: ButtonVariant = 'primary',
    size: ButtonSize = 'md',
    className?: string,
): string {
    return cn(
        'inline-flex cursor-pointer items-center justify-center gap-2 text-center leading-tight whitespace-nowrap no-underline transition-colors disabled:cursor-not-allowed disabled:opacity-50',
        variants[variant],
        sizes[size],
        className,
    );
}

type Common = { variant?: ButtonVariant; size?: ButtonSize };

export function Button({
    variant,
    size,
    className,
    type = 'button',
    ...props
}: Omit<ComponentProps<'button'>, 'size'> & Common) {
    return (
        <button
            type={type}
            className={buttonClass(variant, size, className)}
            {...props}
        />
    );
}

export function ButtonLink({
    variant,
    size,
    className,
    ...props
}: Omit<ComponentProps<typeof Link>, 'size'> & Common) {
    return (
        <Link className={buttonClass(variant, size, className)} {...props} />
    );
}

export function ButtonA({
    variant,
    size,
    className,
    ...props
}: Omit<ComponentProps<'a'>, 'size'> & Common) {
    return <a className={buttonClass(variant, size, className)} {...props} />;
}
