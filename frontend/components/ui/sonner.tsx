'use client'

import { useTheme } from 'next-themes'
import { Toaster as SonnerToaster } from 'sonner'

import { cn } from '@/lib/utils'

type ToasterProps = React.ComponentProps<typeof SonnerToaster>

export function Toaster({
	toastOptions,
	swipeDirections,
	className,
	...props
}: ToasterProps) {
	const { theme = 'system' } = useTheme()

	return (
		<SonnerToaster
			theme={theme as ToasterProps['theme']}
			swipeDirections={swipeDirections}
			className={cn('pointer-events-auto', className)}
			closeButton
			toastOptions={{
				...toastOptions,
				classNames: {
					toast: cn(
						'group toast group-[.toaster]:border-border group-[.toaster]:bg-background group-[.toaster]:text-foreground pointer-events-auto touch-pan-y touch-pan-x',
						toastOptions?.classNames?.toast
					),
					description: cn(
						'group-[.toast]:text-muted-foreground',
						toastOptions?.classNames?.description
					),
					actionButton: cn(
						'group-[.toast]:bg-primary group-[.toast]:text-primary-foreground',
						toastOptions?.classNames?.actionButton
					),
					cancelButton: cn(
						'group-[.toast]:bg-muted group-[.toast]:text-muted-foreground',
						toastOptions?.classNames?.cancelButton
					),
					closeButton: cn(
						'group-[.toast]:bg-background group-[.toast]:text-foreground group-[.toast]:border-border',
						toastOptions?.classNames?.closeButton
					)
				}
			}}
			{...props}
		/>
	)
}
