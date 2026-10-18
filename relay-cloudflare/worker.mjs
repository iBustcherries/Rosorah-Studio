const json = (v, status=200) => Response.json(v,{status,headers:{'Cache-Control':'no-store'}});
const hex = b => [...new Uint8Array(b)].map(x=>x.toString(16).padStart(2,'0')).join('');
export const hash = async s => hex(await crypto.subtle.digest('SHA-256',new TextEncoder().encode(s)));
export const accessKey = token => hash('rosorah-kofi-v2:'+token);
const validToken = t => typeof t==='string' && t.length>=16 && t.length<=256 && !/\s/.test(t);
async function body(request) {
  const reader=request.body?.getReader();if(!reader)throw new Error('body');
  let size=0;const chunks=[];
  for(;;){const {done,value}=await reader.read();if(done)break;size+=value.length;if(size>65536){await reader.cancel();throw new Error('size');}chunks.push(value);}
  const bytes=new Uint8Array(size);let at=0;for(const c of chunks){bytes.set(c,at);at+=c.length;}
  return new TextDecoder('utf-8',{fatal:true}).decode(bytes);
}
export default {
  async fetch(request,env) {
    try {
      const u=new URL(request.url), path=u.pathname;
      if(path==='/health')return json({service:'rosorah-kofi',version:2});
      if(env.API_LIMIT){const r=await env.API_LIMIT.limit({key:request.headers.get('CF-Connecting-IP')||'unknown'});if(!r.success)return json({error:'Try later'},429);}
      let tenant,key;
      if(path.startsWith('/hooks/kofi/')) {
        if(request.method!=='POST')return json({error:'Method'},405);
        tenant=path.slice('/hooks/kofi/'.length);
        if(!/^[a-f0-9]{64}$/.test(tenant))return json({error:'Not found'},404);
      } else {
        const auth=request.headers.get('Authorization')||'';
        if(!/^Bearer [a-f0-9]{64}$/.test(auth))return json({error:'Unauthorized'},401);
        key=auth.slice(7);tenant=await hash(key);
        if(path==='/v1/configure' && env.REGISTER_LIMIT) {
          const r=await env.REGISTER_LIMIT.limit({key:request.headers.get('CF-Connecting-IP')||'unknown'});
          if(!r.success)return json({error:'Try later'},429);
        }
      }
      if(!['/v1/configure','/v1/status','/v1/live','/v2/socket','/v1/account'].includes(path) && !path.startsWith('/hooks/kofi/'))return json({error:'Not found'},404);
      // Set routing identity ourselves; no caller-supplied internal headers are trusted.
      const headers=new Headers(request.headers);headers.set('X-Rosorah-Tenant',tenant);
      headers.set('X-Rosorah-Key',key||'');
      return await env.ACCOUNTS.get(env.ACCOUNTS.idFromName(tenant)).fetch(new Request(request,{headers}));
    } catch {return json({error:'Receiver unavailable'},503);}
  }
};

export class KofiAccount {
  constructor(ctx,env){
    this.ctx=ctx;this.sql=ctx.storage.sql;
    this.sql.exec('CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY CHECK(id=1), tenant TEXT, token_hash TEXT, verified INTEGER, live INTEGER)');
    this.sql.exec('CREATE TABLE IF NOT EXISTS events (seq INTEGER PRIMARY KEY AUTOINCREMENT, payload TEXT, created INTEGER)');
    this.sql.exec('CREATE TABLE IF NOT EXISTS seen (id TEXT PRIMARY KEY, created INTEGER)');
    this.sql.exec('CREATE INDEX IF NOT EXISTS seen_time ON seen(created)');
  }
  row(){return this.sql.exec('SELECT * FROM settings WHERE id=1').toArray()[0];}
  status(origin){const r=this.row();return {owner:'Ko-fi:'+r.tenant,verified:!!r.verified,live:!!r.live,webhook_url:origin+'/hooks/kofi/'+r.tenant,events:[]};}
  send(value){for(const ws of this.ctx.getWebSockets()){try{ws.send(JSON.stringify(value));}catch{try{ws.close(1011,'Disconnected');}catch{}}}}
  async fetch(request){
    try{return await this.handle(request);}catch(e){return json({error:e.message==='size'?'Request too large':'Invalid request'},e.message==='size'?413:400);}
  }
  async handle(request){
    const u=new URL(request.url),p=u.pathname,tenant=request.headers.get('X-Rosorah-Tenant');
    if(p==='/v1/configure' && request.method==='POST'){
      const data=JSON.parse(await body(request)),token=data.verification_token;
      if(!validToken(token) || await accessKey(token)!==request.headers.get('X-Rosorah-Key'))return json({error:'Invalid token'},401);
      const h=await hash(token),r=this.row();
      // Repeat connects never reset live/test state or replace another account.
      if(r && (r.tenant!==tenant || r.token_hash!==h))return json({error:'Account mismatch'},401);
      if(!r)this.sql.exec('INSERT INTO settings VALUES(1,?,?,0,0)',tenant,h);
      return json(this.status(u.origin));
    }
    const r=this.row();if(!r || r.tenant!==tenant)return json({error:'Not connected'},401);
    if(p.startsWith('/hooks/kofi/')){
      if(!request.headers.get('Content-Type')?.startsWith('application/x-www-form-urlencoded'))return json({error:'Content type'},415);
      const data=JSON.parse(new URLSearchParams(await body(request)).get('data'));
      if(!validToken(data?.verification_token) || await hash(data.verification_token)!==r.token_hash)return json({error:'Invalid notification'},401);
      const id=data.message_id;if(typeof id!=='string'||!id.length||id.length>128)return json({error:'Event ID'},400);
      const now=Date.now();
      this.sql.exec('DELETE FROM seen WHERE created<?',now-30*86400000);
      if(this.sql.exec('SELECT id FROM seen WHERE id=?',id).toArray().length)return new Response('OK');
      if(this.sql.exec('SELECT COUNT(*) AS n FROM seen').one().n>=5000)return json({error:'Capacity reached'},503);
      let event=null;
      const test=['is_test','isTest','test'].some(k=>data[k]===true||data[k]==='true'||data[k]===1);
      if(r.live && !test && ['Donation','Subscription'].includes(data.type)){
        const raw=String(data.amount),currency=data.currency;
        if(!/^[0-9]{1,10}(\.[0-9]{1,2})?$/.test(raw)||typeof currency!=='string'||!/^[A-Z]{3}$/.test(currency))return json({error:'Invalid amount'},400);
        const [whole,fraction='']=raw.split('.');const cents=Number(whole)*100+Number(fraction.padEnd(2,'0'));
        if(!Number.isSafeInteger(cents)||cents<=0||cents>100000000000)return json({error:'Invalid amount'},400);
        event={id,seq:1,hundredths:cents,currency,name:data.is_public===true?String(data.from_name||'Anonymous').slice(0,100):'Anonymous',message:data.is_public===true?String(data.message||'').slice(0,1000):'',at_ms:Number.isFinite(Date.parse(data.timestamp))?Date.parse(data.timestamp):now};
      }
      this.ctx.storage.transactionSync(()=>{
        this.sql.exec('INSERT INTO seen VALUES(?,?)',id,now);
        this.sql.exec('UPDATE settings SET verified=1 WHERE id=1');
        if(event){
          this.sql.exec('INSERT INTO events(payload,created) VALUES(?,?)',JSON.stringify(event),now);
          event.seq=this.sql.exec('SELECT last_insert_rowid() AS id').one().id;
        }
        this.sql.exec('DELETE FROM events WHERE created<? OR seq <= (SELECT COALESCE(MAX(seq),0)-1000 FROM events)',now-7*86400000);
      });
      // Live delivery; reconnect history is separately marked and never replays alerts.
      this.send({...this.status(u.origin),events:event?[event]:[]});
      return new Response('OK');
    }
    if(p==='/v1/status' && request.method==='GET')return json(this.status(u.origin));
    if(p==='/v1/live' && request.method==='POST'){
      const d=JSON.parse(await body(request));if(typeof d.live!=='boolean')return json({error:'Invalid mode'},400);
      if(d.live&&!r.verified)return json({error:'Send setup test first'},409);
      this.sql.exec('UPDATE settings SET live=? WHERE id=1',d.live?1:0);const s=this.status(u.origin);this.send(s);return json(s);
    }
    if(p==='/v2/socket' && request.method==='GET'){
      if(request.headers.get('Upgrade')?.toLowerCase()!=='websocket')return json({error:'WebSocket required'},426);
      if(this.ctx.getWebSockets().length)return json({error:'Already connected on another instance'},409);
      const [client,server]=Object.values(new WebSocketPair());this.ctx.acceptWebSocket(server);
      const rows=this.sql.exec('SELECT seq,payload FROM events WHERE created>=? ORDER BY seq',Date.now()-7*86400000).toArray();
      for(let at=0;at<rows.length;at+=64){server.send(JSON.stringify({...this.status(u.origin),historical:true,events:rows.slice(at,at+64).map(r=>({...JSON.parse(r.payload),seq:r.seq}))}));}
      server.send(JSON.stringify(this.status(u.origin)));
      return new Response(null,{status:101,webSocket:client});
    }
    if(p==='/v1/account' && request.method==='DELETE'){
      for(const ws of this.ctx.getWebSockets())ws.close(1000,'Disconnected');
      this.ctx.storage.transactionSync(()=>{this.sql.exec('DELETE FROM settings');this.sql.exec('DELETE FROM seen');this.sql.exec('DELETE FROM events');});return json({ok:true});
    }
    return json({error:'Not found'},404);
  }
  webSocketMessage(ws,message){ws.close(1008,'No application messages expected');}
  webSocketClose(ws,code){try{ws.close(code);}catch{}}
  webSocketError(ws){try{ws.close(1011,'Disconnected');}catch{}}
}
