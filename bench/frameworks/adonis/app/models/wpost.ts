import { BaseModel, column } from '@adonisjs/lucid/orm'

// Isolated table for the write workloads, mirroring the other stacks.
export default class Wpost extends BaseModel {
  static table = 'wposts'

  @column({ isPrimary: true })
  declare id: number

  @column()
  declare title: string

  @column()
  declare views: number
}
