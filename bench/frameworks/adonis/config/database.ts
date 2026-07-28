import { defineConfig } from '@adonisjs/lucid'

export default defineConfig({
  connection: 'postgres',
  connections: {
    postgres: {
      client: 'pg',
      connection: {
        host: '127.0.0.1',
        port: 5433,
        user: 'bench',
        password: 'bench',
        database: 'bench',
      },
      // 5 per worker x 16 workers = 80, the same pool the other Node stack holds.
      pool: { min: 0, max: 5 },
      migrations: { naturalSort: true, paths: ['database/migrations'] },
    },
  },
})
