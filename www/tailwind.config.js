/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    "./app/views/**/*.{slv,html,html.slv,html.erb,etlua}",
    "./app/controllers/**/*.sl",
    "./app/helpers/**/*.sl"
  ],
  theme: {
    extend: {
      colors: {
        primary: '#4f46e5',
        secondary: '#10b981',
      }
    },
  },
  plugins: [
    require('@tailwindcss/typography'),
  ],
}
