/** @type {import('tailwindcss').Config} */
export default {
    content: [
        "./index.html",
        "./src/**/*.{js,ts,jsx,tsx}",
    ],
    theme: {
        extend: {
            colors: {
                // Theme-aware semantic colors
                'bg-primary': 'var(--color-bg-primary)',
                'bg-secondary': 'var(--color-bg-secondary)',
                'bg-tertiary': 'var(--color-bg-tertiary)',
                'bg-elevated': 'var(--color-bg-elevated)',

                'text-primary': 'var(--color-text-primary)',
                'text-secondary': 'var(--color-text-secondary)',
                'text-tertiary': 'var(--color-text-tertiary)',

                'border-primary': 'var(--color-border-primary)',
                'border-secondary': 'var(--color-border-secondary)',

                'accent-primary': 'var(--color-accent-primary)',
                'accent-secondary': 'var(--color-accent-secondary)',

                'success': 'var(--color-success)',
                'error': 'var(--color-error)',
                'warning': 'var(--color-warning)',

                'input-bg': 'var(--color-input-bg)',
                'input-border': 'var(--color-input-border)',
                'input-focus': 'var(--color-input-focus)',
                'hover-bg': 'var(--color-hover-bg)',
                'active-bg': 'var(--color-active-bg)',
            },
            boxShadow: {
                'theme': 'var(--color-shadow)',
            },
            backgroundColor: {
                'overlay': 'var(--color-overlay)',
            },
        },
    },
    plugins: [],
}
