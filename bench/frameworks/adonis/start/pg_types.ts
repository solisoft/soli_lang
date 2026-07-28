/**
 * node-postgres renders int8 as a string by default (precision > 2^53), and the
 * posts/wposts primary keys are bigint. Without this the payload carries
 * `"id":"1"` where every other stack sends `"id":1` — two bytes per row, and no
 * longer the same response. The Express app registers the identical parser.
 */
import pg from 'pg'

pg.types.setTypeParser(20, (value: string) => Number.parseInt(value, 10))
