<?php
namespace App\Models;
use Illuminate\Database\Eloquent\Model;

// Isolated table for the write workloads — mirrors Soli's and Rails' Wpost.
class Wpost extends Model
{
    public $timestamps = false;
    protected $table = 'wposts';
    protected $fillable = ['title', 'views'];
}
