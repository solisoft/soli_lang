<?php

use Illuminate\Foundation\Application;
use Illuminate\Foundation\Configuration\Exceptions;
use Illuminate\Foundation\Configuration\Middleware;
use Illuminate\Http\Request;

return Application::configure(basePath: dirname(__DIR__))
    ->withRouting(
        web: __DIR__.'/../routes/web.php',
        commands: __DIR__.'/../routes/console.php',
        health: '/up',
    )
    ->withMiddleware(function (Middleware $middleware): void {
        // The load generator carries no session or CSRF token; the write routes
        // are a benchmark fixture, not a form endpoint. Sessions and cookies are
        // dropped too so the read rows are not measuring session middleware the
        // other three stacks do not run.
        // Only forgery protection is dropped — the load generator carries no
        // token. The rest of the default web stack stays, so Laravel is measured
        // running what a Laravel app actually runs, as Rails and Django are.
        $middleware->web(remove: [
            \Illuminate\Foundation\Http\Middleware\PreventRequestForgery::class,
        ]);
    })
    ->withExceptions(function (Exceptions $exceptions): void {
        $exceptions->shouldRenderJsonWhen(
            fn (Request $request) => $request->is('api/*'),
        );
    })->create();
