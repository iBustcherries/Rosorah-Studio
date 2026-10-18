import {test} from 'node:test';
import assert from 'node:assert/strict';
import {handle} from './worker.mjs';
const env = {KICK_CLIENT_ID:'registered-app',KICK_CLIENT_SECRET:'server-only-secret'};
const valid = {client_id:env.KICK_CLIENT_ID,grant_type:'authorization_code',code:'one-use-code',code_verifier:'x'.repeat(43),redirect_uri:'http://localhost:17653/oauth/callback'};
const request = (body,extra={}) => new Request('https://auth.example.com/token',{method:'POST',headers:{'Content-Type':'application/json',...extra},body:JSON.stringify(body)});
test('exchanges fixed client and PKCE, only returns token fields',async()=>{
  const response = await handle(request(valid),env,async(url,options)=>{
    assert.equal(url,'https://id.kick.com/oauth/token');
    assert.equal(options.redirect,'manual');
    assert.equal(options.body.get('client_secret'),env.KICK_CLIENT_SECRET);
    assert.equal(options.body.get('code_verifier'),valid.code_verifier);
    return Response.json({access_token:'access',refresh_token:'refresh',expires_in:'3600',client_secret:'never-return-this'});
  });
  assert.equal(response.status,200);assert.equal(response.headers.get('Cache-Control'),'no-store');
  assert.deepEqual(await response.json(),{access_token:'access',refresh_token:'refresh',expires_in:3600,token_type:'Bearer'});
});
test('rejects wrong clients, redirects, missing PKCE, grant types and browser origins before upstream',async()=>{
  let calls=0;const fetcher=()=>{calls++;throw new Error('must not call');};
  for(const patch of [{client_id:'other'},{redirect_uri:'https://evil.example'},{code_verifier:''},{grant_type:'client_credentials'}]){
    assert.equal((await handle(request({...valid,...patch}),env,fetcher)).status,400);
  }
  assert.equal((await handle(request(valid,{Origin:'https://evil.example'}),env,fetcher)).status,403);
  assert.equal(calls,0);
});
test('refresh rotation and omission are preserved',async()=>{
  const input={client_id:env.KICK_CLIENT_ID,grant_type:'refresh_token',refresh_token:'previous'};
  const response=await handle(request(input),env,async(_,options)=>{
    assert.equal(options.body.get('refresh_token'),'previous');
    return Response.json({access_token:'new',refresh_token:'rotated',expires_in:3600});
  });assert.equal((await response.json()).refresh_token,'rotated');
});
test('upstream diagnostics cannot leak secrets and redirects are not forwarded',async()=>{
  const response=await handle(request(valid),env,async()=>new Response('secret-diagnostic',{status:400}));
  assert.equal(await response.text(),'{"error":"authorization_rejected"}');
  const redirected=await handle(request(valid),env,async()=>new Response(null,{status:302,headers:{Location:'https://evil.example'}}));
  assert.equal(redirected.status,502);
});
test('large payload and rate limits stop upstream calls',async()=>{
  const never=()=>{throw new Error('unexpected upstream call');};
  assert.equal((await handle(request({...valid,code:'x'.repeat(17000)}),env,never)).status,400);
  assert.equal((await handle(request(valid),{...env,RATE_LIMIT:{limit:async()=>({success:false})}},never)).status,429);
});
test('missing publisher registration never presents a working login',async()=>{
  assert.equal((await handle(request(valid),{})).status,503);
});
import {verifyKick,resetKickKey} from './worker.mjs';
const {publicKey,privateKey} = await crypto.subtle.generateKey({name:'RSASSA-PKCS1-v1_5',modulusLength:2048,publicExponent:new Uint8Array([1,0,1]),hash:'SHA-256'},true,['sign','verify']);
const spki = new Uint8Array(await crypto.subtle.exportKey('spki',publicKey));
const pem = `-----BEGIN PUBLIC KEY-----\n${btoa(String.fromCharCode(...spki)).match(/.{1,64}/g).join('\n')}\n-----END PUBLIC KEY-----`;
const keyServer = async url => {assert.equal(url,'https://api.kick.com/public/v1/public-key');return Response.json({data:{public_key:pem}});};
const sign = async text => btoa(String.fromCharCode(...new Uint8Array(await crypto.subtle.sign('RSASSA-PKCS1-v1_5',privateKey,new TextEncoder().encode(text)))));
const hook = async (body,patch={}) => {
  const at = new Date().toISOString();
  const headers = {'Kick-Event-Message-Id':'01HZX','Kick-Event-Message-Timestamp':at,'Kick-Event-Type':'channel.followed',
    'Kick-Event-Signature':await sign(`01HZX.${at}.${body}`),...patch};
  return new Request('https://auth.example.com/webhook',{method:'POST',headers,body});
};
test('webhooks need Kick signature and reach only the broadcaster relay',async()=>{
  resetKickKey();
  const body = JSON.stringify({broadcaster:{user_id:42},follower:{username:'Rose'}});
  const published = [];
  const env = {KICK_EVENTS:{idFromName:name=>name,get:name=>({fetch:async(url,options)=>{published.push([name,url,JSON.parse(options.body)]);return new Response(null,{status:204});}})}};
  assert.equal((await handle(await hook(body),env,keyServer)).status,200);
  assert.deepEqual(published.map(p=>[p[0],p[1],p[2].type,p[2].payload.follower.username]),[['42','https://kick-events/publish','channel.followed','Rose']]);
  const signed = await hook(body), tampered = new Request(signed.url,{method:'POST',headers:signed.headers,body:body.replace('Rose','Mallory')});
  assert.equal((await handle(tampered,env,keyServer)).status,401);
  const stale = new Date(Date.now()-2*60*60*1000).toISOString();
  assert.equal((await handle(await hook(body,{'Kick-Event-Message-Timestamp':stale}),env,keyServer)).status,400);
  assert.equal(published.length,1);
});
test('a rotated Kick key is refetched once',async()=>{
  resetKickKey();
  const other = await crypto.subtle.generateKey({name:'RSASSA-PKCS1-v1_5',modulusLength:2048,publicExponent:new Uint8Array([1,0,1]),hash:'SHA-256'},true,['sign','verify']);
  const old = new Uint8Array(await crypto.subtle.exportKey('spki',other.publicKey));
  let calls = 0;
  const rotating = async () => Response.json({data:{public_key:calls++ === 0 ? btoa(String.fromCharCode(...old)) : pem}});
  assert.equal(await verifyKick('a','t','b',await sign('a.t.b'),rotating),true);
  assert.equal(calls,2);
});
test('activity sockets need a valid token for this app and join the owner channel',async()=>{
  const ws = (token) => new Request('https://auth.example.com/events',{headers:{Upgrade:'websocket',...(token?{Authorization:`Bearer ${token}`}:{})}});
  const rooms = [];
  const env = {KICK_CLIENT_ID:'registered-app',KICK_EVENTS:{idFromName:n=>n,get:n=>({fetch:async()=>{rooms.push(n);return new Response(null,{status:204});}})}};
  const kick = client => async url => url.includes('introspect') ? Response.json({data:{active:true,client_id:client}}) : Response.json({data:[{user_id:42}]});
  assert.equal((await handle(ws(''),env,kick('registered-app'))).status,401);
  assert.equal((await handle(ws('tok'),env,kick('other-app'))).status,401);
  await handle(ws('tok'),env,kick('registered-app'));
  assert.deepEqual(rooms,['42']);
  assert.equal((await handle(new Request('https://auth.example.com/events'),env)).status,426);
});
