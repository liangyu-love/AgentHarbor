import type { Config } from "tailwindcss";
import tailwindcssAnimate from "tailwindcss-animate";

const config: Config = {
  darkMode: ["class"],
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
  	extend: {
  		colors: {
  			border: 'hsl(var(--border))',
  			input: 'hsl(var(--input))',
  			ring: 'hsl(var(--ring))',
  			background: 'hsl(var(--background))',
  			foreground: 'hsl(var(--foreground))',
  			primary: {
  				DEFAULT: 'hsl(var(--primary))',
  				foreground: 'hsl(var(--primary-foreground))'
  			},
  			secondary: {
  				DEFAULT: 'hsl(var(--secondary))',
  				foreground: 'hsl(var(--secondary-foreground))',
  				// Already carries its own alpha, so it must not be wrapped in
  				// `hsl(...)` again -- see the `--accent` note in index.css.
  				overlay: 'hsl(var(--secondary-overlay))'
  			},
  			destructive: {
  				DEFAULT: 'hsl(var(--destructive))',
  				foreground: 'hsl(var(--destructive-foreground))'
  			},
  			muted: {
  				DEFAULT: 'hsl(var(--muted))',
  				foreground: 'hsl(var(--muted-foreground))'
  			},
  			accent: {
  				DEFAULT: 'hsl(var(--accent))',
  				foreground: 'hsl(var(--accent-foreground))',
  				// Translucent twin; see the `--accent` note in index.css.
  				overlay: 'hsl(var(--accent-overlay))'
  			},
  			popover: {
  				DEFAULT: 'hsl(var(--popover))',
  				foreground: 'hsl(var(--popover-foreground))'
  			},
  			card: {
  				DEFAULT: 'hsl(var(--card))',
  				foreground: 'hsl(var(--card-foreground))'
  			},
  			sidebar: {
  				DEFAULT: 'hsl(var(--sidebar-background))',
  				foreground: 'hsl(var(--sidebar-foreground))',
  				primary: 'hsl(var(--sidebar-primary))',
  				'primary-foreground': 'hsl(var(--sidebar-primary-foreground))',
  				accent: 'hsl(var(--sidebar-accent))',
  				'accent-foreground': 'hsl(var(--sidebar-accent-foreground))',
  				border: 'hsl(var(--sidebar-border))',
  				ring: 'hsl(var(--sidebar-ring))'
  			}
  		},
  		borderRadius: {
  			lg: 'var(--radius)',
  			md: 'calc(var(--radius) - 4px)',
  			sm: 'calc(var(--radius) - 8px)',
  			// Tailwind 4's `rounded-xs`. Tailwind 3 renamed it to `rounded-sm`,
  			// so the call site using it compiled to no rule at all.
  			xs: '0.125rem',
  			'2xl': '1rem',
  			'3xl': '1.25rem'
  		},
  		boxShadow: {
  			bento: '0 8px 30px -10px rgba(0,0,0,0.45)',
  			'bento-light': '0 2px 16px -4px rgba(0,0,0,0.07)',
  			'bento-inner': 'inset 0 1px 0 rgba(255,255,255,0.04)',
  			// Tailwind 4 renamed `shadow-sm` -> `shadow-xs` and added a new,
  			// even lighter `shadow-2xs`. Tailwind 3 has neither name, so every
  			// `shadow-xs` / `shadow-2xs` in this codebase compiled to no rule at
  			// all and the cards lost their elevation. Both names are restored
  			// with their Tailwind 4 values; `2xs` sits one step below `xs`.
  			'2xs': '0 1px rgb(0 0 0 / 0.05)',
  			xs: '0 1px 2px 0 rgb(0 0 0 / 0.05)'
  		},
  		// Fractional and sub-`4` steps used throughout the UI (`h-4.5`, `w-7.5`,
  		// `py-0.2`, `h-13`, `w-84`, ...). Tailwind 3 ships only 0.5/1.5/2.5/3.5
  		// and even integers, so everything else compiled to nothing.
  		spacing: {
  			'0.2': '0.05rem',
  			'4.5': '1.125rem',
  			'5.5': '1.375rem',
  			'6.5': '1.625rem',
  			'7.5': '1.875rem',
  			'8.5': '2.125rem',
  			'13': '3.25rem',
  			'84': '21rem'
  		},
  		transitionDuration: {
  			// `duration-400` is not in the Tailwind 3 scale; the sheet open
  			// animation silently fell back to `duration-150`.
  			'400': '400ms'
  		},
  		backdropBlur: {
  			// Tailwind 4's `backdrop-blur-xs`; Tailwind 3 starts at `sm`.
  			xs: '2px'
  		},
  		backgroundImage: {
  			'glass-gradient': 'linear-gradient(135deg, rgba(255,255,255,0.04) 0%, rgba(255,255,255,0) 100%)',
  			'gradient-radial': 'radial-gradient(ellipse at 30% 40%, var(--tw-gradient-from) 0%, var(--tw-gradient-via) 35%, var(--tw-gradient-to) 70%)'
  		},
  		fontFamily: {
  			// Poster headlines and large numerals; CJK falls back to the system face.
  			display: [
  				'"Bricolage Grotesque"',
  				'Bahnschrift',
  				'"Segoe UI Variable Display"',
  				'system-ui',
  				'sans-serif'
  			],
  			sans: [
  				'system-ui',
  				'-apple-system',
  				'BlinkMacSystemFont',
  				'Segoe UI',
  				'Roboto',
  				'Helvetica Neue',
  				'Arial',
  				'Noto Sans',
  				'sans-serif',
  				'Apple Color Emoji',
  				'Segoe UI Emoji',
  				'Segoe UI Symbol',
  				'Noto Color Emoji'
  			],
  			mono: [
  				'ui-monospace',
  				'SFMono-Regular',
  				'Menlo',
  				'Monaco',
  				'Consolas',
  				'Liberation Mono',
  				'Courier New',
  				'monospace'
  			]
  		},
  		keyframes: {
  			rainbow: {
  				'0%': { backgroundPosition: '0%' },
  				'100%': { backgroundPosition: '200%' },
  			},
  		},
  		animation: {
  			rainbow: 'rainbow 2s linear infinite',
  		},
  	}
  },
  plugins: [tailwindcssAnimate],
};

export default config;
