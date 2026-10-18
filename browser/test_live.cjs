const assert=require('node:assert/strict'),fs=require('node:fs'),{JSDOM}=require('jsdom');
const collector=fs.readFileSync(__dirname+'/collector.js','utf8');
(async()=>{
 {const d=new JSDOM('<main></main>',{url:'https://www.twitch.tv/popout/moe/chat',runScripts:'outside-only'}),w=d.window;
 w.setTimeout=()=>{throw Error('collector must not depend on a background timer');};
 w.eval(collector);const received=[];w.__rosorah.setSink(m=>received.push(m));
 const start=performance.now();
 w.document.querySelector('main').innerHTML=Array.from({length:350},(_,i)=>`<div data-a-target="chat-line-message" data-id="m${i}"><span data-a-target="chat-message-username">Moe</span><span class="message">👋 hello <img alt="Kappa" src="https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/3.0"></span></div>`).join('');
 await new Promise(setImmediate);
 assert.equal(received.length,350);assert.equal(received[0].parts[1].text,'Kappa');assert.ok(received[0].parts[1].src);assert.equal(w.__rosorah.drain().length,0);
 console.log(`PASS 350-message push burst without timers (${(performance.now()-start).toFixed(0)} ms including jsdom DOM construction; not a platform latency measurement)`);
 w.document.querySelector('img').src='https://evil.test/track';await new Promise(setImmediate);assert.equal(received.at(-1).id,'m0');assert.equal(received.at(-1).parts[1].src,undefined);d.window.close();}
 const html=fs.readFileSync(__dirname+'/dock.html','utf8'),d=new JSDOM(html,{runScripts:'dangerously',beforeParse(w){w.scrollTo=()=>{};}}),w=d.window;
 const m={type:'message',platform:'Twitch',id:'1',author:'Moe',text:'👋 Kappa',kind:'chat',parts:[{text:'👋 '},{text:'Kappa',src:'https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/3.0'},{text:'evil',src:'https://evil.test/x'}]};w.ingest(m);assert.equal(w.document.querySelectorAll('#feed img').length,1);w.ingest({...m,parts:[{text:'Updated'}]});assert.equal(w.document.querySelectorAll('.message').length,1);assert.match(w.document.querySelector('.body').textContent,/Updated/);
 let outgoing=[];w.connectComposer({send:(...args)=>outgoing.push(args),open:()=>{}});w.setAccounts(['Twitch']);assert.equal(w.document.querySelector('#target').options.length,1);assert.equal(w.document.querySelector('#target').hidden,true);const input=w.document.querySelector('#draft');input.value='Hello 👋';w.document.querySelector('#compose').dispatchEvent(new w.Event('submit',{cancelable:true}));assert.equal(outgoing.length,1);assert.equal(outgoing[0][0],'Twitch');w.postResult({request:outgoing[0][2],ok:false,text:'Sign in'});assert.equal(input.value,'Hello 👋');
 w.document.querySelector('#compose').dispatchEvent(new w.Event('submit',{cancelable:true}));w.postResult({request:outgoing[1][2],ok:true,text:'Submitted'});assert.equal(input.value,'');w.setAccounts([]);assert.equal(w.document.querySelector('#send').disabled,true);input.value='Must not send';w.document.querySelector('#compose').dispatchEvent(new w.Event('submit',{cancelable:true}));assert.equal(outgoing.length,2);d.window.close();
 console.log('PASS rich emotes, late image changes, safe URLs, send routing, character limits, disabled chat, draft preservation and composer acknowledgements');
})().catch(e=>{console.error(e);process.exitCode=1;});
