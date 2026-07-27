@extends('layouts.app')
@section('content')
<h1>Posts</h1>
<table>
@foreach ($items as $item)
<tr><td>{{ $item->id }}</td><td>{{ $item->title }}</td><td>{{ $item->views }}</td></tr>
@endforeach
</table>
@endsection
