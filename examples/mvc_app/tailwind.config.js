/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    "./app/views/**/*.{erb,html,html.erb,etlua}",
    "./app/controllers/**/*.soli",
    "./app/helpers/**/*.soli"
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
