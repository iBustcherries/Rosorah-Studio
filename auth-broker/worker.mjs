// Publisher-owned Kick token exchange and activity relay. No tokens are stored and no arbitrary URLs are fetched.
const REDIRECT = 'http://localhost:17653/oauth/callback';
const headers = {'Content-Type':'application/json', 'Cache-Control':'no-store', 'Pragma':'no-cache'};
const reply = (status, value) => new Response(JSON.stringify(value), {status, headers});
async function boundedText(response, limit = 16384) {
  const reader = response.body?.getReader();
  if (!reader) throw new Error('empty');
  let size = 0; const chunks = [];
  try {
    for (;;) {
      const {done,value} = await reader.read(); if (done) break;
      size += value.byteLength; if (size > limit) throw new Error('size');
      chunks.push(value);
    }
  } finally { await reader.cancel().catch(()=>{}); }
  const bytes = new Uint8Array(size); let offset = 0;
  for (const chunk of chunks) {bytes.set(chunk,offset); offset += chunk.byteLength;}
  return new TextDecoder().decode(bytes);
}
const boundedJson = async response => JSON.parse(await boundedText(response));
const upstream = (fetcher, url, options = {}) => fetcher(url, {redirect:'manual',signal:AbortSignal.timeout(12000),...options,
  headers:{'Accept':'application/json','User-Agent':'RosorahStudio-KickAuth/1.0',...options.headers}});

// Kick signs each webhook as base64(RSA-PKCS1v15-SHA256("<message id>.<timestamp>.<raw body>")).
// The key is fetched from Kick and refetched once on a mismatch, because Kick may rotate it.
let kickKey = null;
async function loadKey(fetcher) {
  const response = await upstream(fetcher, 'https://api.kick.com/public/v1/public-key');
  if (!response.ok) {await response.body?.cancel(); throw new Error('public key unavailable');}
  const pem = (await boundedJson(response))?.data?.public_key;
  if (typeof pem !== 'string') throw new Error('public key unavailable');
  const der = Uint8Array.from(atob(pem.replace(/-----[^-]+-----|\s/g,'')), c => c.charCodeAt(0));
  return crypto.subtle.importKey('spki', der, {name:'RSASSA-PKCS1-v1_5',hash:'SHA-256'}, false, ['verify']);
}
export async function verifyKick(id, timestamp, body, signature, fetcher = fetch) {
  let sig;
  try {sig = Uint8Array.from(atob(signature), c => c.charCodeAt(0));} catch {return false;}
  const data = new TextEncoder().encode(`${id}.${timestamp}.${body}`);
  for (const fresh of [false, true]) {
    if (!kickKey || fresh) kickKey = await loadKey(fetcher);
    if (await crypto.subtle.verify('RSASSA-PKCS1-v1_5', kickKey, sig, data)) return true;
  }
  return false;
}
export function resetKickKey() {kickKey = null;}

async function webhook(request, env, fetcher) {
  if (request.method !== 'POST') return reply(405,{error:'method_not_allowed'});
  const id = request.headers.get('Kick-Event-Message-Id') || '', at = request.headers.get('Kick-Event-Message-Timestamp') || '';
  const type = request.headers.get('Kick-Event-Type') || '', signature = request.headers.get('Kick-Event-Signature') || '';
  if (!/^[A-Za-z0-9]{1,64}$/.test(id) || !/^[a-z.]{1,64}$/.test(type) || !signature || at.length > 64) return reply(400,{error:'invalid_request'});
  // Old deliveries are refused so a captured webhook can't be replayed later.
  const sent = Date.parse(at);
  if (!Number.isFinite(sent) || Math.abs(Date.now() - sent) > 60 * 60 * 1000) return reply(400,{error:'stale_event'});
  let body;
  try {body = await boundedText(request, 65536);} catch {return reply(413,{error:'too_large'});}
  let valid;
  try {valid = await verifyKick(id, at, body, signature, fetcher);} catch {return reply(503,{error:'key_unavailable'});}
  if (!valid) return reply(401,{error:'invalid_signature'});
  let payload;
  try {payload = JSON.parse(body);} catch {return reply(400,{error:'invalid_request'});}
  const channel = payload?.broadcaster?.user_id;
  if (!Number.isSafeInteger(channel) || !env.KICK_EVENTS) return reply(200,{ok:true});
  const room = env.KICK_EVENTS.get(env.KICK_EVENTS.idFromName(String(channel)));
  await room.fetch('https://kick-events/publish', {method:'POST',body:JSON.stringify({type,id,at,payload})});
  return reply(200,{ok:true});
}

// The desktop app opens a WebSocket here with its Kick access token. The token is checked
// with Kick and is never stored; the socket joins only the token owner's own channel.
async function events(request, env, fetcher) {
  if (request.headers.get('Upgrade')?.toLowerCase() !== 'websocket') return reply(426,{error:'websocket_required'});
  if (request.headers.has('Origin')) return reply(403,{error:'origin_not_allowed'});
  if (!env.KICK_EVENTS || !env.KICK_CLIENT_ID) return reply(503,{error:'publisher_setup_required'});
  const token = /^Bearer ([A-Za-z0-9._~+/=-]{1,4096})$/.exec(request.headers.get('Authorization') || '')?.[1];
  if (!token) return reply(401,{error:'token_required'});
  if (env.RATE_LIMIT) {
    const result = await env.RATE_LIMIT.limit({key:request.headers.get('CF-Connecting-IP') || 'unknown'});
    if (!result.success) return reply(429,{error:'rate_limited'});
  }
  let user;
  try {
    const auth = {headers:{Authorization:`Bearer ${token}`}};
    const check = await upstream(fetcher, 'https://id.kick.com/oauth/token/introspect', {method:'POST',...auth});
    const identity = check.ok ? await boundedJson(check) : (await check.body?.cancel(), null);
    if (identity?.data?.active !== true || identity.data.client_id !== env.KICK_CLIENT_ID) return reply(401,{error:'invalid_token'});
    const me = await upstream(fetcher, 'https://api.kick.com/public/v1/users', auth);
    user = me.ok ? (await boundedJson(me))?.data?.[0]?.user_id : (await me.body?.cancel(), null);
  } catch {return reply(502,{error:'upstream_unavailable'});}
  if (!Number.isSafeInteger(user)) return reply(401,{error:'invalid_token'});
  const room = env.KICK_EVENTS.get(env.KICK_EVENTS.idFromName(String(user)));
  return room.fetch('https://kick-events/connect', {headers:{Upgrade:'websocket'}});
}

export async function handle(request, env, fetcher = fetch) {
  const url = new URL(request.url);
  if (url.pathname === '/webhook') return webhook(request, env, fetcher);
  if (url.pathname === '/events') return events(request, env, fetcher);
  if (url.pathname !== '/token') return reply(404,{error:'not_found'});
  if (request.method !== 'POST') return reply(405,{error:'method_not_allowed'});
  if (!env.KICK_CLIENT_ID || !env.KICK_CLIENT_SECRET) return reply(503,{error:'publisher_setup_required'});
  // A desktop client calls this endpoint directly; it is not a browser AJAX API.
  if (request.headers.has('Origin')) return reply(403,{error:'origin_not_allowed'});
  if (request.headers.get('Content-Type')?.split(';')[0].trim().toLowerCase() !== 'application/json') return reply(415,{error:'json_required'});
  if (env.RATE_LIMIT) {
    const result = await env.RATE_LIMIT.limit({key:request.headers.get('CF-Connecting-IP') || 'unknown'});
    if (!result.success) return reply(429,{error:'rate_limited'});
  }
  let input;
  try {input = await boundedJson(request);} catch {return reply(400,{error:'invalid_request'});}
  if (!input || input.client_id !== env.KICK_CLIENT_ID) return reply(400,{error:'invalid_client'});
  const text = (value,max=4096) => typeof value === 'string' && value.length > 0 && value.length <= max;
  const form = new URLSearchParams({client_id:env.KICK_CLIENT_ID,client_secret:env.KICK_CLIENT_SECRET});
  if (input.grant_type === 'authorization_code') {
    if (input.redirect_uri !== REDIRECT || !text(input.code) || typeof input.code_verifier !== 'string' || !/^[A-Za-z0-9._~-]{43,128}$/.test(input.code_verifier)) return reply(400,{error:'invalid_request'});
    form.set('grant_type','authorization_code'); form.set('code',input.code);
    form.set('code_verifier',input.code_verifier); form.set('redirect_uri',REDIRECT);
  } else if (input.grant_type === 'refresh_token' && text(input.refresh_token)) {
    form.set('grant_type','refresh_token'); form.set('refresh_token',input.refresh_token);
  } else return reply(400,{error:'unsupported_grant_type'});
  let response;
  try {
    response = await fetcher('https://id.kick.com/oauth/token', {
      // Workers reject redirect:'error'; 'manual' plus the 3xx check below still never follows one.
      method:'POST',body:form,redirect:'manual',signal:AbortSignal.timeout(12000),
      headers:{'Content-Type':'application/x-www-form-urlencoded','Accept':'application/json','User-Agent':'RosorahStudio-KickAuth/1.0'}
    });
  } catch (error) {
    // Logs only the failure reason; never the request body, code or tokens.
    console.log(JSON.stringify({kick_token:'fetch_failed',grant:input.grant_type,reason:String(error?.name||'')+': '+String(error?.message||'')}));
    return reply(502,{error:'upstream_unavailable'});
  }
  if (!response.ok) {
    console.log(JSON.stringify({kick_token:'rejected',grant:input.grant_type,status:response.status,cf_ray:response.headers.get('cf-ray'),server:response.headers.get('server'),content_type:response.headers.get('content-type')}));
    await response.body?.cancel();
    const unavailable = response.status >= 500 || (response.status >= 300 && response.status < 400) || response.type === 'opaqueredirect';
    return reply(response.status === 429 ? 429 : unavailable ? 502 : 400,
      {error:unavailable ? 'upstream_unavailable' : response.status === 429 ? 'rate_limited' : 'authorization_rejected'});
  }
  let value;
  try {value = await boundedJson(response);} catch {return reply(502,{error:'invalid_upstream_response'});}
  const expiry = Number(value?.expires_in);
  if (!text(value?.access_token) || !Number.isFinite(expiry) || expiry <= 0) return reply(502,{error:'invalid_upstream_response'});
  const result = {access_token:value.access_token,token_type:'Bearer',expires_in:expiry};
  if (text(value.refresh_token)) result.refresh_token = value.refresh_token;
  return reply(200,result);
}

// One relay per Kick channel. Hibernation keeps idle sockets free; "ping" is answered without waking it.
export class KickEvents {
  constructor(ctx) {
    this.ctx = ctx; this.seen = [];
    ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair('ping','pong'));
  }
  async fetch(request) {
    const path = new URL(request.url).pathname;
    if (path === '/connect') {
      if (this.ctx.getWebSockets().length >= 8) return reply(429,{error:'too_many_connections'});
      const [client, server] = Object.values(new WebSocketPair());
      this.ctx.acceptWebSocket(server);
      return new Response(null,{status:101,webSocket:client});
    }
    if (path === '/publish') {
      const text = await request.text();
      // Kick retries deliveries; the app also ignores repeats.
      const id = JSON.parse(text).id;
      if (this.seen.includes(id)) return new Response(null,{status:204});
      this.seen.push(id); if (this.seen.length > 512) this.seen.shift();
      for (const socket of this.ctx.getWebSockets()) {try {socket.send(text);} catch {}}
      return new Response(null,{status:204});
    }
    return reply(404,{error:'not_found'});
  }
  webSocketMessage() {}
  webSocketClose(socket, code) {try {socket.close(code === 1005 ? 1000 : code);} catch {}}
}
export default {fetch:(request,env)=>handle(request,env)};
