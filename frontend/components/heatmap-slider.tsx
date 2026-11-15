'use client'

import * as React from 'react'
import * as Slider from '@radix-ui/react-slider'

type Props = {
	id?: string
	value: number
	min?: number
	max?: number
	step?: number
	onChange: (v: number) => void
}

export default function HeatmapSlider({
	id,
	value,
	min = 1,
	max = 30,
	step = 1,
	onChange
}: Props) {
	return (
		<Slider.Root
			id={id}
			className="relative flex items-center select-none touch-none w-full h-5"
			value={[value]}
			min={min}
			max={max}
			step={step}
			onValueChange={val => onChange(val[0])}
		>
			<Slider.Track className="relative bg-muted h-2 w-full rounded-full">
				<Slider.Range className="absolute bg-orange-500 h-full rounded-full" />
			</Slider.Track>
			<Slider.Thumb className="block w-5 h-5 bg-background border-2 border-orange-500 rounded-full shadow-md hover:scale-110 transition-transform" />
		</Slider.Root>
	)
}
