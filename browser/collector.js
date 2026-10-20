(() => {
  if (window.__rosorah) return;
  const host = location.hostname;
  const platform = host === 'www.youtube.com' ? 'YouTube' : (host === 'kick.com' || host === 'www.kick.com') ? 'Kick' : (host === 'www.tiktok.com' || host === 'tiktok.com') ? 'TikTok' : 'Twitch';
  const seen = new Map(), queue = [];
  let sink = null, initial = true;
  const opened = Date.now();
  const selector = platform === 'YouTube'
    ? 'yt-live-chat-text-message-renderer,yt-live-chat-paid-message-renderer,yt-live-chat-paid-sticker-renderer,yt-live-chat-membership-item-renderer,yt-live-chat-sponsorships-gift-purchase-announcement-renderer,yt-live-chat-sponsorships-gift-redemption-announcement-renderer'
    : platform === 'Kick'
    ? '#chatroom-messages [data-index],[data-chat-entry]'
    : platform === 'TikTok'
    ? '[data-e2e$="message"]'
    : '[data-a-target="chat-line-message"]';
  const safeImage = src => {
    try { const u = new URL(src); return u.protocol === 'https:' && (/(^|\.)(tiktokcdn|tiktokcdn-us|ibyteimg)\.com$/.test(u.hostname) || ['static-cdn.jtvnw.net','yt3.ggpht.com','yt3.googleusercontent.com','www.gstatic.com','fonts.gstatic.com','i.ytimg.com','files.kick.com'].includes(u.hostname) || (u.hostname==='www.youtube.com' && u.pathname.startsWith('/s/gaming/emoji/'))) ? u.href : ''; } catch { return ''; }
  };
  function parts(el) {
    if (!el) return [];
    const result = [];
    function walk(n) {
      if (n.nodeType === 3) result.push({text:n.textContent});
      else if (n.nodeType === 1 && n.tagName === 'IMG') {
        const src = safeImage(n.currentSrc || n.src), text = n.alt || n.getAttribute('aria-label') || '';
        result.push(src ? {text,src} : {text});
      } else for (const child of n.childNodes) walk(child);
    }
    walk(el);
    // Bound both IPC size and per-message rendering cost, while preserving text fallback.
    let left = 5000;
    return result.slice(0,128).map(p => { const text=p.text.slice(0,left);left-=text.length;return p.src ? {text,src:p.src.slice(0,2048)} : {text}; });
  }
  const text = el => parts(el).map(p=>p.text).join('');
  // Name colour, badges and highlights, for the chat widget. Badges are pictures where the page
  // has them, otherwise role names. Pages change; anything not found is simply left out.
  const hex = c => { const m=/rgba?\((\d+),\s*(\d+),\s*(\d+)/.exec(c||''); return m ? '#'+m.slice(1,4).map(n=>(+n).toString(16).padStart(2,'0')).join('') : ''; };
  const ROLES=[['broadcaster',/broadcaster|owner|streamer|\bhost\b/i],['moderator',/moderator|\bmod\b/i],['vip',/\bvip\b/i],['founder',/founder/i],['staff',/\bstaff\b|\badmin/i],['og',/^og\b/i],['verified',/verified|partner/i],['subscriber',/subscri|\bsub\b/i],['member',/member/i]];
  const role = label => (ROLES.find(([,p])=>p.test(label||''))||[''])[0];
  const badge = b => { const img=b.tagName==='IMG'?b:b.querySelector('img'); const src=img?safeImage(img.currentSrc||img.src):''; return src || role(b.getAttribute('type')||b.getAttribute('title')||b.getAttribute('aria-label')||(img&&img.alt)||''); };
  function look(el, nameEl, body) {
    let color='', badges=[], highlight='';
    if (platform==='YouTube') {
      const type=el.getAttribute('author-type')||'';
      color={owner:'#ffd600',moderator:'#5e84f1',member:'#2ba640'}[type]||'';
      badges=[...el.querySelectorAll('yt-live-chat-author-badge-renderer')].map(badge);
    } else if (platform==='Twitch') {
      const name=el.querySelector('.chat-author__display-name,[data-a-target="chat-message-username"]');
      color=name?hex(name.style.color||getComputedStyle(name).color):'';
      badges=[...el.querySelectorAll('img.chat-badge,[data-a-target="chat-badge"] img')].map(badge);
      if (/\bcheer\d+\b/i.test(body) || el.querySelector('[class*="cheer" i]')) highlight='paid';
      else if (el.querySelector('[class*="first-message" i],[class*="first-time" i]') || /first[- ]time chat/i.test(el.textContent||'')) highlight='first';
      else if (/highlighted-message|channel-points/i.test((el.className||'')+' '+(el.parentElement?.className||''))) highlight='highlight';
    } else if (platform==='Kick') {
      color=nameEl?hex(nameEl.style.color||getComputedStyle(nameEl).color):'';
      // Badges sit in the header, before the name.
      const head=nameEl?.parentElement;
      if (head) badges=[...head.querySelectorAll('img,svg,[title],[aria-label]')].filter(b=>b!==nameEl&&!nameEl.contains(b)&&!b.closest('svg>*')).map(badge);
    }
    badges=[...new Set(badges.filter(Boolean))].slice(0,6);
    return {color,badges,highlight};
  }
  // The chatter's profile picture, where the chat page shows one (YouTube and TikTok).
  const photo = el => {
    const img = platform==='YouTube' ? el.querySelector('#author-photo img') : platform==='TikTok' ? [...el.querySelectorAll('img')].find(i=>/tiktokcdn/.test(i.currentSrc||i.src)) : null;
    const src = img ? (img.currentSrc||img.src||'') : '';
    return src.startsWith('https://') ? src.slice(0,2048) : '';
  };
  const emit = item => { if (sink) sink(item); else {queue.push(item);if(queue.length>500)queue.shift();} };
  const hashId=(prefix,key)=>{let h=0;for(const c of key)h=(Math.imul(h,31)+c.codePointAt(0))|0;return prefix+(h>>>0).toString(36)+'-'+key.length;};
  // TikTok hashes its class names per release, so only data-e2e hooks and visible text are used.
  // Joins and likes arrive constantly and are skipped; gifts, follows, subscriptions and shares are kept.
  const TIKTOK_EVENTS=[['gift',/\b(sent|gifted)\b/i],['follow',/\bfollowed\b/i],['subscribe',/\bsubscribed\b/i],['share',/\bshared\b/i]];
  function inspectTikTok(el) {
    if(el.parentElement?.closest(selector))return;
    const e2e=el.getAttribute('data-e2e')||'';
    if(/enter|join|like/i.test(e2e))return;
    const nameEl=el.querySelector('[data-e2e="message-owner-name"],[data-e2e*="owner-name"],[data-e2e*="user-name"],[data-e2e*="nickname"]');
    const author=text(nameEl).trim().slice(0,100);
    let dropped=false;
    const fragments=parts(el).filter(p=>{if(!dropped&&author&&p.text.trim()===author){dropped=true;return false;}return true;}).slice(0,128);
    const body=fragments.map(p=>p.text).join('').replace(/^\s*:\s*/,'').trim().slice(0,5000);
    if(!body&&!fragments.some(p=>p.src))return;
    const chat=e2e==='chat-message'||/comment/i.test(e2e);
    let activity_kind=/gift/i.test(e2e)?'gift':'';
    if(!chat&&!activity_kind)for(const [kind,pattern] of TIKTOK_EVENTS)if(pattern.test(body)){activity_kind=kind;break;}
    if(!chat&&!activity_kind)return;
    // A gift streak updates one row ("x2", "x3"); keep one id so it alerts once.
    const id=hashId('tt-',author+'\n'+activity_kind+'\n'+body.replace(/\s*[x×]\s*\d+\s*$/i,''));
    const value=JSON.stringify([author,fragments]);
    if(seen.get(id)===value)return;
    const first=!seen.has(id);
    remember(id,value);
    if(!first&&activity_kind)return;
    emit({type:'message',platform,id,author:author||'Viewer',author_id:'',author_photo:photo(el),text:body,parts:fragments,kind:activity_kind?'activity':'chat',activity_kind,historical:initial || Date.now()-opened<5000});
  }
  function inspect(el) {
    if(platform==='TikTok'){inspectTikTok(el);return;}
    let id=el.id || el.getAttribute('data-id') || el.getAttribute('data-chat-entry');
    if (!id) {if(!el.__rosorahId)el.__rosorahId=crypto.randomUUID();id=el.__rosorahId;}
    if((platform==='YouTube' && (el.hasAttribute('is-deleted') || el.querySelector('[is-deleted]'))) || (platform==='Twitch' && el.querySelector('[data-a-target="chat-deleted-message-placeholder"],[data-test-selector="chat-deleted-message-placeholder"]'))){
      if(seen.get(id)!=='deleted'){remember(id,'deleted');emit({type:'delete',platform,id});}return;
    }
    const nameEl=el.querySelector(platform==='YouTube'?'#author-name':platform==='Kick'?'button[title][data-prevent-expand],.chat-entry-username,[data-chat-entry-user-id]':'[data-a-target="chat-message-username"]');
    const author=text(nameEl).trim().slice(0,100);
    let fragments=[];
    if(platform==='YouTube') {
      for(const sel of ['#purchase-amount','#header-subtext','#message','#sticker']) {
        const next=parts(el.querySelector(sel));
        if(next.length) {if(fragments.length)fragments.push({text:' · '});fragments.push(...next);}
      }
    } else if(platform==='Kick') {
      // Current chat: the message body follows the "name:" header; badges and timestamp stay in the header.
      const sep=[...el.querySelectorAll('span[aria-hidden="true"]')].find(s=>s.textContent.trim()===':');
      for(let n=sep?.parentElement?.nextElementSibling;n;n=n.nextElementSibling)fragments.push(...parts(n));
      // Older chat: a Kick message is split into several content spans (text and emotes), in order.
      if(!sep)el.querySelectorAll('.chat-entry-content').forEach(span=>{
        if(span.parentElement?.closest('.chat-entry-content'))return;
        const next=parts(span);
        if(next.length)fragments.push(...next);
      });
    } else fragments=parts(el.querySelector('[data-a-target="chat-line-message-body"],[data-test-selector="chat-line-message-body"]') || el.querySelector('.message'));
    fragments=fragments.slice(0,128);
    const body=fragments.map(p=>p.text).join('').trim().slice(0,5000);
    const kind=platform==='YouTube' && el.tagName!=='YT-LIVE-CHAT-TEXT-MESSAGE-RENDERER'?'activity':'chat';
    if(!body && !fragments.some(p=>p.src) && kind==='chat')return;
    // Kick's virtualized list reuses row elements for other messages, so identify rows by content.
    if(platform==='Kick' && el.hasAttribute('data-index')){
      id=hashId('kick-',author+'\n'+(el.querySelector('span')?.textContent||'')+'\n'+body);
    }
    // Images may acquire their source after the message text first arrives.
    const value=JSON.stringify([author,fragments]);
    if(seen.get(id)===value || seen.get(id)==='deleted')return;
    remember(id,value);
    const activity_kind = ({'YT-LIVE-CHAT-PAID-MESSAGE-RENDERER':'super_chat','YT-LIVE-CHAT-PAID-STICKER-RENDERER':'super_sticker','YT-LIVE-CHAT-MEMBERSHIP-ITEM-RENDERER':'membership','YT-LIVE-CHAT-SPONSORSHIPS-GIFT-PURCHASE-ANNOUNCEMENT-RENDERER':'gift','YT-LIVE-CHAT-SPONSORSHIPS-GIFT-REDEMPTION-ANNOUNCEMENT-RENDERER':'gift'})[el.tagName] || '';
    // Twitch exposes the sender's login, which moderation actions need; display names can differ.
    const author_id=platform==='Twitch'?(el.getAttribute('data-a-user')||el.querySelector('[data-a-user]')?.getAttribute('data-a-user')||'').slice(0,25):'';
    emit({type:'message',platform,id:id.slice(0,200),author:author||'Viewer',author_id,author_photo:photo(el),...look(el,nameEl,body),text:body||text(el).trim().slice(0,2000),parts:fragments,kind,activity_kind,historical:initial || Date.now()-opened<5000});
  }
  function remember(id,value){seen.set(id,value);while(seen.size>3000)seen.delete(seen.keys().next().value);}
  function scan(root=document) {
    if(root.nodeType===3)root=root.parentElement;
    if(!root)return;
    const own=root.closest?.(selector);
    if(own)inspect(own);
    root.querySelectorAll?.(selector).forEach(inspect);
  }
  // MutationObserver already batches in a microtask: no throttled timeout, interval or animation frame.
  new MutationObserver(changes=>{
    const roots=new Set();
    for(const change of changes){
      if(change.type==='childList'){roots.add(change.target);for(const node of change.addedNodes)roots.add(node);}
      else roots.add(change.target);
    }
    roots.forEach(scan);
  }).observe(document.documentElement,{childList:true,subtree:true,characterData:true,attributes:true,attributeFilter:['is-deleted','src','srcset']});
  // YouTube puts the emotes the signed-in viewer can use (YouTube's own and the channel's) into the
  // page data; the streamer's emote picker in Rosorah lists them.
  function youtubeEmotes(){
    if(platform!=='YouTube')return;
    const items=[],ids=new Set();
    const visit=v=>{
      if(!v||typeof v!=='object'||items.length>=200)return;
      if(Array.isArray(v)){v.forEach(visit);return;}
      if(v.emojiId&&v.isCustomEmoji&&Array.isArray(v.shortcuts)&&v.image){
        const thumbs=v.image.thumbnails||[],src=safeImage(thumbs.length?thumbs[thumbs.length-1].url:''),name=String(v.shortcuts[0]||'').slice(0,100);
        if(src&&name&&!ids.has(v.emojiId)){ids.add(v.emojiId);items.push({name,src:src.slice(0,2048)});}
        return;
      }
      for(const k in v)visit(v[k]);
    };
    for(const s of document.scripts){
      const t=s.textContent||'',i=t.indexOf('ytInitialData');if(i<0)continue;
      const start=t.indexOf('{',i),end=t.lastIndexOf('}');if(start<0||end<start)continue;
      try{visit(JSON.parse(t.slice(start,end+1)));}catch(e){}
    }
    if(items.length)emit({type:'emote_list',platform,items});
  }
  window.__rosorah={drain:()=>{scan();return queue.splice(0);},setSink:fn=>{sink=fn;queue.splice(0).forEach(emit);},scan};
  scan();
  youtubeEmotes();
  initial = false;
})();
