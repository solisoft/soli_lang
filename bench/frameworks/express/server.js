// Express + EJS + PostgreSQL — les trois charges appariées.
// Express n'a ni couche de vue ni ORM : les deux ont été ajoutés, et la colonne
// du tableau de résultats est nommée en conséquence.
//
// Pool de 5 par worker x 16 workers = 80 connexions, identique au 16x5 de Puma.
const cluster = require('node:cluster');
const N = 16;

if (cluster.isPrimary) {
  for (let i = 0; i < N; i++) cluster.fork();
} else {
  const express = require('express');
  const ejs = require('ejs');
  const pg = require('pg');
  const { Pool } = pg;

  // node-postgres rend les int8 en chaine par defaut (precision > 2^53).
  // Rails/pluck et Soli rendent des entiers : sans ca la charge utile differe
  // de 2 octets par ligne et n'est plus appariee.
  pg.types.setTypeParser(20, (v) => parseInt(v, 10));

  const app = express();
  const pool = new Pool({
    host: '127.0.0.1', port: 5433, user: 'bench', password: 'bench', database: 'bench',
    max: 5,
  });
  // objets nommes {id,title,views} — meme forme que le pluck de Soli
  const DB_QUERY = { text: 'SELECT id, title, views FROM posts' };

  // ORM, pour que la ligne DB compare deux ORM et non un ORM a une requete
  // ecrite a la main. `raw: true` = projection sans instancier de modeles,
  // exactement ce que font le pluck de Rails et celui de Soli.
  const { Sequelize, DataTypes } = require('sequelize');
  const sequelize = new Sequelize('bench', 'bench', 'bench', {
    host: '127.0.0.1', port: 5433, dialect: 'postgres', logging: false,
    pool: { max: 5, min: 0 },
  });
  const PostModel = sequelize.define('Post', {
    title: DataTypes.STRING,
    views: DataTypes.INTEGER,
  }, { tableName: 'posts', timestamps: false });
  const ORM_PROJECTION = { attributes: ['id', 'title', 'views'], raw: true };

  const rows = () =>
    Array.from({ length: 50 }, (_, i) => ({ id: i + 1, title: `Post title ${i + 1}`, views: (i + 1) * 7 }));

  const layout = ejs.compile(
    '<!DOCTYPE html>\n<html lang="en">\n<head>\n<meta charset="UTF-8">\n<title><%= title %></title>\n</head>\n<body>\n<%- body %>\n</body>\n</html>\n'
  );
  const list = ejs.compile(
    '<h1>Posts</h1>\n<table>\n<% items.forEach(function(item){ %>\n<tr><td><%= item.id %></td><td><%= item.title %></td><td><%= item.views %></td></tr>\n<% }); %>\n</table>\n'
  );

  app.get('/json', (req, res) => res.json(rows()));
  app.get('/template', (req, res) =>
    res.type('html').send(layout({ title: 'Posts', body: list({ items: rows() }) }))
  );
  app.get('/db', async (req, res) => {
    try {
      const r = await pool.query(DB_QUERY);
      res.json(r.rows);
    } catch (e) {
      res.status(500).json({ error: String(e) });
    }
  });

  // lecture DB puis rendu HTML — /db et /template en une seule requete
  app.get('/db-template', async (req, res) => {
    try {
      const r = await pool.query(DB_QUERY);
      res.type('html').send(layout({ title: 'Posts', body: list({ items: r.rows }) }));
    } catch (e) {
      res.status(500).send(String(e));
    }
  });

  // ---- Ecritures : une operation par requete, sur `wposts` (800 000 lignes) ----
  // Cle tiree au hasard dans le meme intervalle 1..800000 que les deux autres.
  const WPost = sequelize.define('Wpost', {
    title: DataTypes.STRING,
    views: DataTypes.INTEGER,
  }, { tableName: 'wposts', timestamps: false });
  const WPOOL = 800000;
  const wkey = () => Math.floor(Math.random() * WPOOL) + 1;

  app.post('/w', async (req, res) => {
    try { await WPost.create({ title: 'Post title 0', views: 7 }); res.sendStatus(201); }
    catch (e) { res.status(500).send(String(e)); }
  });
  app.patch('/w', async (req, res) => {
    try { await WPost.update({ views: 42 }, { where: { id: wkey() } }); res.sendStatus(200); }
    catch (e) { res.status(500).send(String(e)); }
  });
  app.delete('/w', async (req, res) => {
    try { await WPost.destroy({ where: { id: wkey() } }); res.sendStatus(200); }
    catch (e) { res.status(500).send(String(e)); }
  });

  // Les deux memes charges, via l'ORM au lieu du pilote brut.
  app.get('/db-orm', async (req, res) => {
    try {
      res.json(await PostModel.findAll(ORM_PROJECTION));
    } catch (e) {
      res.status(500).json({ error: String(e) });
    }
  });
  app.get('/db-template-orm', async (req, res) => {
    try {
      const rows = await PostModel.findAll(ORM_PROJECTION);
      res.type('html').send(layout({ title: 'Posts', body: list({ items: rows }) }));
    } catch (e) {
      res.status(500).send(String(e));
    }
  });

  app.listen(5097);
}
