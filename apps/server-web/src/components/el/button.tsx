import { Link } from 'react-router-dom';
import type { ComponentProps } from 'react';
import { cn } from '@/lib/utils';

/** The desktop app's buttons (`.btn` in app.css). `default` is its plain raised button. */
export type ButtonVariant = 'default' | 'primary' | 'outline' | 'amber' | 'danger' | 'light' | 'ghost';
export type ButtonSize = 'sm' | 'md' | 'lg';

const variants: Record<ButtonVariant, string> = {
    default: '',
    primary: 'primary',
    outline: 'btn-outline',
    amber: 'amber-outline',
    danger: 'danger',
    light: 'light',
    ghost: 'ghost',
};

const sizes: Record<ButtonSize, string> = {
    sm: 'small',
    md: '',
    lg: 'tall',
};

export function buttonClass(variant: ButtonVariant = 'primary', size: ButtonSize = 'md', className?: string): string {
    return cn('btn', variants[variant], sizes[size], className);
}

type Common = { variant?: ButtonVariant; size?: ButtonSize };

export function Button({ variant, size, className, type = 'button', ...props }: Omit<ComponentProps<'button'>, 'size'> & Common) {
    return <button type={type} className={buttonClass(variant, size, className)} {...props} />;
}

export function ButtonLink({ variant, size, className, ...props }: Omit<ComponentProps<typeof Link>, 'size'> & Common) {
    return <Link className={buttonClass(variant, size, className)} {...props} />;
}

export function ButtonA({ variant, size, className, ...props }: Omit<ComponentProps<'a'>, 'size'> & Common) {
    return <a className={buttonClass(variant, size, className)} {...props} />;
}
