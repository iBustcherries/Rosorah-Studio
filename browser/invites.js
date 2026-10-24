// Runs on your Twitch Stream Manager page (in Rosorah's background chat browser) and reports a
// co-stream invite showing there (Stream Together, shared chat, Guest Star), so Rosorah can offer
// Accept and Decline under Chat. Twitch has no API for invites, so this reads the page: an
// Accept/Join button near text about an invite. Only the invite's own text leaves the page.
(() => {
  if (window.__rosorah) return;
  const queue = [];
  let sink = null, last = '', found = null;
  const emit = item => { if (sink) sink(item); else { queue.push(item); if (queue.length > 20) queue.shift(); } };
  const label = b => ((b.innerText || '') + ' ' + (b.getAttribute('aria-label') || '')).replace(/\s+/g, ' ').trim();
  // Twitch's pop-up actions can be links as well as buttons.
  const CONTROLS = 'button,a,[role="button"],[role="link"]';
  const ACCEPT = /^(accept|join|join now|accept invite|accept invitation)\b/i;
  const DECLINE = /^(decline|reject|ignore|no thanks|not now)\b/i;
  // Twitch's invite pop-up ("<name> wants to collaborate. Click to accept!") has Accept and Close.
  const CLOSE = /^(close|dismiss)\b/i;
  const ABOUT = /invit|stream together|shared chat|guest star|collab|co-?stream|wants to stream/i;
  const control = b => { const l = label(b); return l.length < 40 && (ACCEPT.test(l) || DECLINE.test(l) || CLOSE.test(l)); };
  function find() {
    for (const accept of document.querySelectorAll(CONTROLS)) {
      // Not whether it's drawn: Twitch keeps its pop-up in the page but may not draw it in a
      // narrow window, and a click on it still works.
      if (!ACCEPT.test(label(accept)) || accept.disabled) continue;
      // The smallest surrounding box that says what the invite is.
      for (let box = accept.parentElement, depth = 0; box && depth < 8; box = box.parentElement, depth++) {
        const text = (box.innerText || '').replace(/\s+/g, ' ').trim();
        if (text.length > 600) break;
        if (ABOUT.test(text)) {
          const buttons = [...box.querySelectorAll(CONTROLS)];
          const decline = buttons.find(b => DECLINE.test(label(b)));
          const close = buttons.find(b => CLOSE.test(label(b)));
          // What the invite says, without its Accept/Decline/Close buttons. The message itself can
          // be a button too (Twitch's pop-up is clickable), so only those controls are dropped, and
          // each piece of text is kept apart ("…collaborate." and "now").
          const parts = [], walk = document.createTreeWalker(box, NodeFilter.SHOW_TEXT);
          for (let n; (n = walk.nextNode());) {
            const owner = n.parentElement && n.parentElement.closest(CONTROLS + ',script,style,svg');
            if (!(owner && box.contains(owner) && (owner.matches('script,style,svg') || control(owner)))) parts.push(n.data);
          }
          const said = parts.join(' ').replace(/\s+/g, ' ')
            .replace(/\s*click (here )?to accept!?/i, '').replace(/\s+(now|just now|\d+\s*[smh]( ago)?)$/i, '').trim();
          // Twitch's Accept is a link to the Stream Together backstage page, which Rosorah opens in
          // your own browser instead of this hidden one.
          const link = accept.tagName === 'A' ? accept.href : '';
          return { box, accept, link, decline: decline || close, closeOnly: !decline && !!close, text: said || text };
        }
      }
    }
    return null;
  }
  // Twitch's pop-up closes itself after a few seconds unless the pointer rests on it.
  function hold(box) {
    for (const type of ['pointerover', 'pointerenter', 'mouseover', 'mouseenter'])
      box.dispatchEvent(new MouseEvent(type, { bubbles: type.endsWith('over'), relatedTarget: document.body }));
  }
  // The page shows your own stream; nothing here needs to play.
  const quiet = () => document.querySelectorAll('video,audio').forEach(v => { v.muted = true; if (!v.paused) v.pause(); });
  // An invite that went away without being answered here stays reported as gone, so Rosorah can
  // keep it on screen a while and point you to Twitch. One answered here is left alone while
  // Twitch's pop-up for it is still closing, so it doesn't come back as a new invite.
  let shown = '', link = '', answered = '';
  function scan() {
    quiet();
    // Signed out, the Stream Manager shows Twitch's login form ("You must be logged in…").
    const signin = (location.hostname === 'www.twitch.tv' && location.pathname === '/login')
      || !!document.querySelector('[data-a-target="passport-modal"],[data-a-target="login-username-input"]');
    found = signin ? null : find();
    const said = found ? found.text.slice(0, 300) : '';
    if (said !== answered) answered = '';
    const live = !!found && !answered;
    if (live) {
      hold(found.box);
      if (said !== shown) { shown = said; link = ''; }
      link = found.link || link;
    }
    const gone = !found && !!shown;
    const text = live || gone ? shown : '';
    const state = JSON.stringify([signin, text, link, gone, live && !!found.decline, live && found.closeOnly]);
    if (state !== last) {
      last = state;
      emit({ type: 'invite', platform: 'Twitch', signin, text, link: text ? link : '', gone, can_decline: live && !!found.decline, close_only: live && found.closeOnly });
    }
  }
  // Answered or dismissed: forget it, and ignore its pop-up until Twitch takes it away.
  function done() { answered = shown || (found ? found.text.slice(0, 300) : ''); shown = ''; link = ''; }
  let pending = false;
  new MutationObserver(() => { if (!pending) { pending = true; setTimeout(() => { pending = false; scan(); }, 500); } })
    .observe(document.documentElement, { childList: true, subtree: true, characterData: true });
  window.__rosorah = {
    scan,
    drain: () => { scan(); return queue.splice(0); },
    setSink: fn => { sink = fn; queue.splice(0).forEach(emit); },
    answer: what => { scan(); const b = found && (what === 'accept' ? found.accept : found.decline); if (b) { done(); b.click(); } setTimeout(scan, 800); return !!b; },
    // Done here (or opened elsewhere): Twitch's pop-up is closed too, if it's still up.
    forget: () => { scan(); const b = found && found.closeOnly && found.decline; done(); if (b) b.click(); scan(); setTimeout(scan, 800); },
  };
  scan();
})();
