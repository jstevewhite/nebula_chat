/** @type {import('tailwindcss').Config} */
export default {
    content: [
        "./index.html",
        "./src/**/*.{js,ts,jsx,tsx}",
    ],
    theme: {
        extend: {
            // Theme colors are accessed via arbitrary values: bg-[var(--color-bg-primary)]
            // This approach works reliably without custom class generation
        },
    },
    plugins: [],
}
