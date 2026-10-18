import {test} from 'node:test';
import assert from 'node:assert/strict';
import {Miniflare} from 'miniflare';
import {accessKey,hash} from './worker.mjs';

test('token setup, live/missed activity, privacy, dedup and isolation',async()=>{
 const mf=new Miniflare({modules:true,scriptPath:'worker.mjs',compatibilityDate:'2026-08-01',durableObjects:{ACCOUNTS:{className:'KofiAccount',useSQLite:true}}});
 const token='example-private-kofi-token';const key=await accessKey(token),tenant=await hash(key);
 const headers={Authorization:'Bearer '+key};
 const api=(path,method='GET',data)=>mf.dispatchFetch('https://relay.example'+path,{method,headers:{...headers,'Content-Type':'application/json'},body:data?JSON.stringify(data):undefined});
 const notify=async(id,fields={})=>mf.dispatchFetch('https://relay.example/hooks/kofi/'+tenant,{method:'POST',headers:{'Content-Type':'application/x-www-form-urlencoded'},body:new URLSearchParams({data:JSON.stringify({verification_token:token,message_id:id,type:'Donation',amount:'12.34',currency:'USD',is_public:true,from_name:'Viewer',message:'thanks',timestamp:'2026-10-11T21:42:00Z',email:'private@example.com',...fields})})});
 const open=async()=>{const r=await mf.dispatchFetch('https://relay.example/v2/socket',{headers:{...headers,Upgrade:'websocket'}});assert.equal(r.status,101);r.webSocket.accept();const queue=[];r.webSocket.addEventListener('message',e=>queue.push(JSON.parse(e.data)));return {ws:r.webSocket,queue};};
 const wait=async(test)=>{for(let i=0;i<100;i++){if(test())return;await new Promise(r=>setTimeout(r,10));}assert.fail('Timed out');};
 try{
  assert.equal((await api('/v1/configure','POST',{verification_token:token})).status,200);
  assert.equal((await api('/v1/live','POST',{live:true})).status,409);
  let c=await open();await wait(()=>c.queue.length>0);
  await notify('setup');await wait(()=>c.queue.some(x=>x.verified));
  assert.equal(c.queue.flatMap(x=>x.events).length,0);
  assert.equal((await api('/v1/live','POST',{live:true})).status,200);
  await notify('live');await wait(()=>c.queue.some(x=>x.events.length));
  assert.equal(c.queue.flatMap(x=>x.events)[0].hundredths,1234);
  assert.equal(c.queue.find(x=>x.events.length).historical,undefined);
  await notify('live');await notify('setup');
  assert.equal((await notify('bad',{amount:'1e999999'})).status,400);
  assert.equal((await notify('wrong',{verification_token:'wrong-token-1234567'})).status,401);
  c.ws.close();await new Promise(r=>setTimeout(r,50));
  await notify('missed',{is_public:false});
  c=await open();await wait(()=>c.queue.some(x=>x.historical));
  const backlog=c.queue.filter(x=>x.historical).flatMap(x=>x.events);
  assert.equal(backlog.length,2);assert.equal(backlog[1].name,'Anonymous');assert.equal(backlog[1].message,'');assert.equal(backlog[1].at_ms,Date.parse('2026-10-11T21:42:00Z'));
  assert.ok(!JSON.stringify(backlog).includes('private@example.com'));
  const other=await mf.dispatchFetch('https://relay.example/v1/status',{headers:{Authorization:'Bearer '+'a'.repeat(64)}});assert.equal(other.status,401);
  assert.equal((await api('/v1/configure','POST',{verification_token:token})).status,200);
  assert.equal((await (await api('/v1/status')).json()).live,true);
  assert.equal((await api('/v1/account','DELETE')).status,200);
  assert.equal((await api('/v1/status')).status,401);
 }finally{await mf.dispose();}
});
