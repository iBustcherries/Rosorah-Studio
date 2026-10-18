"""Rosorah Ko-fi receiver: private per-user delivery, durable dedup, no payment processing."""
import asyncio, base64, hashlib, hmac, json, os, re, secrets, sqlite3, time
from decimal import Decimal, InvalidOperation
from pathlib import Path
from aiohttp import web

MAX_EVENTS = 10000
MAX_SEEN = 50000
RETENTION = 7 * 86400

def digest(value):
    return hashlib.sha256(value.encode()).hexdigest()

class Store:
    def __init__(self, path):
        Path(path).parent.mkdir(parents=True, exist_ok=True)
        self.db = sqlite3.connect(path)
        os.chmod(path, 0o600)
        self.db.row_factory = sqlite3.Row
        self.db.executescript('''PRAGMA journal_mode=WAL;
        CREATE TABLE IF NOT EXISTS tenants(id TEXT PRIMARY KEY, key_hash TEXT UNIQUE NOT NULL,
          verify_hash TEXT, verified INTEGER DEFAULT 0, live INTEGER DEFAULT 0,
          ack INTEGER DEFAULT 0, created INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS events(seq INTEGER PRIMARY KEY AUTOINCREMENT,
          tenant TEXT NOT NULL, event_id TEXT NOT NULL, payload TEXT NOT NULL, created INTEGER NOT NULL,
          UNIQUE(tenant,event_id));
        CREATE INDEX IF NOT EXISTS delivery ON events(tenant,seq);
        CREATE TABLE IF NOT EXISTS seen(tenant TEXT, event_id TEXT, created INTEGER,
          PRIMARY KEY(tenant,event_id));''')
        self.db.commit()
    def create(self, public_url):
        tenant, key = secrets.token_hex(16), secrets.token_urlsafe(32)
        with self.db:
            self.db.execute('INSERT INTO tenants(id,key_hash,created) VALUES(?,?,?)', (tenant,digest(key),int(time.time())))
        return base64.urlsafe_b64encode(json.dumps({'version':1,'url':public_url,'key':key},separators=(',',':')).encode()).decode().rstrip('=')
    def tenant(self, key):
        return self.db.execute('SELECT * FROM tenants WHERE key_hash=?',(digest(key),)).fetchone()
    def status(self, tenant, public_url):
        row=self.db.execute('SELECT * FROM tenants WHERE id=?',(tenant,)).fetchone()
        return {'owner':'Ko-fi:'+tenant,'verified':bool(row['verified']),'live':bool(row['live']),
                'webhook_url':public_url+'/hooks/kofi/'+tenant}
    def prune(self):
        now=int(time.time())
        with self.db:
            self.db.execute('DELETE FROM events WHERE created<?',(now-RETENTION,))
            self.db.execute('DELETE FROM seen WHERE created<?',(now-30*86400,))
    def receive(self, tenant, data):
        row=self.db.execute('SELECT * FROM tenants WHERE id=?',(tenant,)).fetchone()
        token=data.get('verification_token')
        if not row or not isinstance(token,str) or not row['verify_hash'] or not hmac.compare_digest(row['verify_hash'],digest(token)):
            raise web.HTTPUnauthorized(text='Invalid notification')
        event_id=data.get('message_id')
        if not isinstance(event_id,str) or not 1<=len(event_id)<=128:
            raise web.HTTPBadRequest(text='Missing event ID')
        # Setup mode receives verification only. Ko-fi test payloads have no guaranteed test flag.
        with self.db:
            self.db.execute('UPDATE tenants SET verified=1 WHERE id=?',(tenant,))
            seen=self.db.execute('SELECT 1 FROM seen WHERE tenant=? AND event_id=?',(tenant,event_id)).fetchone()
            if seen:return
            if self.db.execute('SELECT COUNT(*) FROM seen WHERE tenant=?',(tenant,)).fetchone()[0]>=MAX_SEEN:
                raise web.HTTPServiceUnavailable(text='Notification quota reached',headers={'Retry-After':'3600'})
            now=int(time.time())
            self.db.execute('INSERT INTO seen VALUES(?,?,?)',(tenant,event_id,now))
            if not row['live'] or any(data.get(k) in (True,'true',1) for k in ('is_test','isTest','test')):return
            if data.get('type') not in ('Donation','Subscription'):return
            try:
                raw=str(data['amount'])
                if not re.fullmatch(r'[0-9]{1,12}(?:\.[0-9]{1,2})?',raw):raise ValueError()
                amount=Decimal(raw)
                cents=amount*100
                if not amount.is_finite() or cents!=cents.to_integral_value() or not 0<cents<=100_000_000_000:raise ValueError()
                currency=data['currency']
                if not isinstance(currency,str) or not re.fullmatch('[A-Z]{3}',currency):raise ValueError()
            except (KeyError,ValueError,InvalidOperation):
                raise web.HTTPBadRequest(text='Invalid amount or currency')
            count=self.db.execute('SELECT COUNT(*) FROM events WHERE tenant=? AND seq>?',(tenant,row['ack'])).fetchone()[0]
            if count>=MAX_EVENTS:raise web.HTTPServiceUnavailable(text='Delivery queue full',headers={'Retry-After':'60'})
            public=data.get('is_public') is True
            name=data.get('from_name','Anonymous') if public else 'Anonymous'
            message=data.get('message','') if public else ''
            payload={'id':event_id,'name':str(name)[:100],'message':str(message or '')[:1000],
                     'hundredths':int(cents),'currency':currency,'at_ms':now*1000}
            self.db.execute('INSERT INTO events(tenant,event_id,payload,created) VALUES(?,?,?,?)',(tenant,event_id,json.dumps(payload),now))

@web.middleware
async def errors(request,handler):
    try:return await handler(request)
    except web.HTTPException:raise
    except (json.JSONDecodeError,UnicodeDecodeError,ValueError,TypeError):raise web.HTTPBadRequest(text='Invalid request')
    except Exception:
        # Do not log payloads, headers, tokens, or webhook URLs.
        raise web.HTTPServiceUnavailable(text='Receiver temporarily unavailable') from None

def validate_origin(public_url):
    if not public_url.startswith('https://') or public_url.endswith('/'):
        raise ValueError('PUBLIC_URL must be an HTTPS origin without a trailing slash')
    from urllib.parse import urlsplit
    u=urlsplit(public_url)
    if not u.hostname or u.path or u.query or u.fragment or u.username or u.password:raise ValueError('PUBLIC_URL must be an origin')
    return public_url

def make_app(path, public_url, wait_seconds=20):
    validate_origin(public_url)
    app=web.Application(client_max_size=64*1024,middlewares=[errors])
    store=Store(path); signals={}; active=set()
    def wake(tenant):
        if tenant in signals:signals[tenant].set()
    def auth(request):
        header=request.headers.get('Authorization','')
        if not header.startswith('Bearer ') or len(header)>256:raise web.HTTPUnauthorized()
        row=store.tenant(header[7:])
        if not row:raise web.HTTPUnauthorized()
        return row
    async def health(request):return web.json_response({'service':'rosorah-kofi','version':1})
    async def status(request):return web.json_response(store.status(auth(request)['id'],public_url))
    async def configure(request):
        tenant=auth(request)['id'];data=await request.json()
        if not isinstance(data,dict):raise web.HTTPBadRequest()
        token=data.get('verification_token')
        if not isinstance(token,str) or not 16<=len(token)<=256 or any(c.isspace() for c in token):raise web.HTTPBadRequest(text='Invalid verification token')
        with store.db:
            store.db.execute('UPDATE tenants SET verify_hash=?,verified=0,live=0 WHERE id=?',(digest(token),tenant))
        wake(tenant)
        return web.json_response(store.status(tenant,public_url))
    async def enable(request):
        row=auth(request);data=await request.json()
        if not isinstance(data,dict):raise web.HTTPBadRequest()
        live=data.get('live')
        if not isinstance(live,bool):raise web.HTTPBadRequest()
        if live and not row['verified']:raise web.HTTPConflict(text='Send a Ko-fi test notification first')
        with store.db:store.db.execute('UPDATE tenants SET live=? WHERE id=?',(int(live),row['id']))
        wake(row['id']);return web.json_response(store.status(row['id'],public_url))
    async def hook(request):
        if request.content_type!='application/x-www-form-urlencoded':raise web.HTTPUnsupportedMediaType()
        form=await request.post();raw=form.get('data')
        if not isinstance(raw,str):raise web.HTTPBadRequest()
        data=json.loads(raw)
        if not isinstance(data,dict):raise web.HTTPBadRequest()
        tenant=request.match_info['tenant'];store.receive(tenant,data);wake(tenant)
        return web.Response(text='OK')
    async def events(request):
        row=auth(request);tenant=row['id']
        if tenant in active:raise web.HTTPTooManyRequests(text='One delivery connection per account')
        active.add(tenant);signal=signals.setdefault(tenant,asyncio.Event());signal.clear()
        def batch():
            ack=store.db.execute('SELECT ack FROM tenants WHERE id=?',(tenant,)).fetchone()[0]
            return [dict(json.loads(r['payload']),seq=r['seq']) for r in store.db.execute('SELECT seq,payload FROM events WHERE tenant=? AND seq>? ORDER BY seq LIMIT 64',(tenant,ack))]
        try:
            rows=batch()
            if not rows:
                try:await asyncio.wait_for(signal.wait(),wait_seconds)
                except asyncio.TimeoutError:pass
                rows=batch()
            return web.json_response(dict(store.status(tenant,public_url),events=rows))
        finally:active.discard(tenant);signals.pop(tenant,None)
    async def ack(request):
        tenant=auth(request)['id'];data=await request.json()
        if not isinstance(data,dict):raise web.HTTPBadRequest()
        seq=data.get('seq')
        if type(seq) is not int or seq<0:raise web.HTTPBadRequest()
        maximum=store.db.execute('SELECT MAX(seq) FROM events WHERE tenant=?',(tenant,)).fetchone()[0] or 0
        if seq>maximum:raise web.HTTPBadRequest()
        with store.db:store.db.execute('UPDATE tenants SET ack=MAX(ack,?) WHERE id=?',(seq,tenant))
        return web.json_response({'ok':True})
    async def revoke(request):
        tenant=auth(request)['id']
        with store.db:
            for table in ('events','seen'):store.db.execute(f'DELETE FROM {table} WHERE tenant=?',(tenant,))
            store.db.execute('DELETE FROM tenants WHERE id=?',(tenant,))
        wake(tenant);return web.json_response({'ok':True})
    async def maintenance(app):
        async def prune():
            while True:store.prune();await asyncio.sleep(3600)
        task=asyncio.create_task(prune())
        yield
        task.cancel()
        try:await task
        except asyncio.CancelledError:pass
        store.db.close()
    app.cleanup_ctx.append(maintenance)
    app.router.add_get('/health',health)
    app.router.add_get('/v1/status',status)
    app.router.add_post('/v1/configure',configure)
    app.router.add_post('/v1/live',enable)
    app.router.add_get('/v1/events',events)
    app.router.add_post('/v1/ack',ack)
    app.router.add_delete('/v1/account',revoke)
    app.router.add_post('/hooks/kofi/{tenant:[a-f0-9]{32}}',hook)
    return app

if __name__=='__main__':
    import argparse
    parser=argparse.ArgumentParser();parser.add_argument('--create-connection',action='store_true');args=parser.parse_args()
    path=os.environ.get('DATABASE','/data/relay.sqlite3');url=validate_origin(os.environ['PUBLIC_URL'].rstrip('/'))
    os.umask(0o077)
    if args.create_connection:
        store=Store(path);print(store.create(url));store.db.close()
    else:
        web.run_app(make_app(path,url),host='0.0.0.0',port=int(os.environ.get('PORT','8080')),
                    access_log=None,handler_cancellation=True,print=None)
