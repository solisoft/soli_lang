// 16 workers, matching every other stack. AdonisJS ships a single-process
// server; Node's cluster is how a production deployment scales it, and it is
// the same shape the Express app uses.
import cluster from 'node:cluster'

const N = Number(process.env.WORKERS || 16)

if (cluster.isPrimary) {
  for (let i = 0; i < N; i++) cluster.fork()
  cluster.on('exit', () => cluster.fork())
} else {
  await import('./server.js')
}
