<?php
use App\Http\Controllers\BenchController;
use Illuminate\Support\Facades\Route;

Route::get('/json',         [BenchController::class, 'jsonOnly']);
Route::get('/template',     [BenchController::class, 'templateOnly']);
Route::get('/db',           [BenchController::class, 'dbJson']);
Route::get('/db-template',  [BenchController::class, 'dbTemplate']);
Route::get('/db-hydrated',  [BenchController::class, 'dbJsonHydrated']);
Route::post('/w',           [BenchController::class, 'wCreate']);
Route::patch('/w',          [BenchController::class, 'wUpdate']);
Route::delete('/w',         [BenchController::class, 'wDelete']);
