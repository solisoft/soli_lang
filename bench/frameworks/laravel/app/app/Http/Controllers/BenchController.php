<?php
namespace App\Http\Controllers;

use App\Models\Post;
use App\Models\Wpost;

// The same matched workloads the other three stacks serve, on Eloquent + Blade.
class BenchController extends Controller
{
    private const WPOOL = 800000;

    /** 50 in-memory rows, identical to the other stacks. */
    private function rows(): array
    {
        $out = [];
        for ($i = 1; $i <= 50; $i++) {
            $out[] = (object) ['id' => $i, 'title' => "Post title {$i}", 'views' => $i * 7];
        }
        return $out;
    }

    /**
     * Projection without hydrating models — Eloquent's own escape hatch, and the
     * analogue of Rails' pluck, Soli's pluck and Sequelize's `raw: true`.
     * Hydrating 50 Post models instead is measured separately as a reference.
     */
    private function dbRows()
    {
        return Post::query()->toBase()->select('id', 'title', 'views')->get();
    }

    public function jsonOnly()     { return response()->json($this->rows()); }
    public function templateOnly() { return view('posts.list', ['title' => 'Posts', 'items' => $this->rows()]); }
    public function dbJson()       { return response()->json($this->dbRows()); }
    public function dbTemplate()   { return view('posts.list', ['title' => 'Posts', 'items' => $this->dbRows()]); }

    /** Reference: the canonical Eloquent form, which does instantiate 50 models. */
    public function dbJsonHydrated()
    {
        return response()->json(Post::query()->select('id', 'title', 'views')->get());
    }

    // ---- Writes: one operation per request against the 800,000-row table ----
    public function wCreate()
    {
        Wpost::create(['title' => 'Post title 0', 'views' => 7]);
        return response('', 201);
    }

    public function wUpdate()
    {
        Wpost::where('id', random_int(1, self::WPOOL))->update(['views' => 42]);
        return response('', 200);
    }

    public function wDelete()
    {
        Wpost::where('id', random_int(1, self::WPOOL))->delete();
        return response('', 200);
    }
}
