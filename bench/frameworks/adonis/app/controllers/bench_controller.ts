import type { HttpContext } from '@adonisjs/core/http'
import Post from '#models/post'
import Wpost from '#models/wpost'
import db from '@adonisjs/lucid/services/db'

const WPOOL = 800_000

export default class BenchController {
  /** 50 in-memory rows, identical to the other stacks. */
  private rows() {
    return Array.from({ length: 50 }, (_, i) => ({
      id: i + 1,
      title: `Post title ${i + 1}`,
      views: (i + 1) * 7,
    }))
  }

  /**
   * Projection without hydrating models — the Lucid analogue of Rails' pluck,
   * Soli's pluck, Sequelize's raw:true, Eloquent's toBase() and Django's
   * .values(). Hydrating 50 Post models instead is /db-hydrated.
   */
  private dbRows() {
    return db.from('posts').select('id', 'title', 'views')
  }

  async jsonOnly({ response }: HttpContext) {
    return response.json(this.rows())
  }

  async templateOnly({ view }: HttpContext) {
    return view.render('posts/list', { title: 'Posts', items: this.rows() })
  }

  async dbJson({ response }: HttpContext) {
    return response.json(await this.dbRows())
  }

  async dbTemplate({ view }: HttpContext) {
    return view.render('posts/list', { title: 'Posts', items: await this.dbRows() })
  }

  /** Reference: the form that does instantiate 50 Lucid models. */
  async dbHydrated({ response }: HttpContext) {
    const rows = await Post.query().select('id', 'title', 'views')
    return response.json(rows.map((r) => ({ id: r.id, title: r.title, views: r.views })))
  }

  // ---- Writes: one operation per request against the 800,000-row table ----
  async wCreate({ response }: HttpContext) {
    await Wpost.create({ title: 'Post title 0', views: 7 })
    return response.status(201).send('')
  }

  async wUpdate({ response }: HttpContext) {
    const id = Math.floor(Math.random() * WPOOL) + 1
    await Wpost.query().where('id', id).update({ views: 42 })
    return response.status(200).send('')
  }

  async wDelete({ response }: HttpContext) {
    const id = Math.floor(Math.random() * WPOOL) + 1
    await Wpost.query().where('id', id).delete()
    return response.status(200).send('')
  }
}
