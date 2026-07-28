import router from '@adonisjs/core/services/router'

const BenchController = () => import('#controllers/bench_controller')

router.get('/json', [BenchController, 'jsonOnly'])
router.get('/template', [BenchController, 'templateOnly'])
router.get('/db', [BenchController, 'dbJson'])
router.get('/db-template', [BenchController, 'dbTemplate'])
router.get('/db-hydrated', [BenchController, 'dbHydrated'])
router.post('/w', [BenchController, 'wCreate'])
router.patch('/w', [BenchController, 'wUpdate'])
router.delete('/w', [BenchController, 'wDelete'])
