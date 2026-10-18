// Called only by an explicit Send action. Never retried automatically.
(text => {
  const youtube=location.hostname==='www.youtube.com';
  const input=document.querySelector(youtube ? 'yt-live-chat-text-input-field-renderer #input[contenteditable="true"]' : '[data-a-target="chat-input"][contenteditable="true"],textarea[data-a-target="chat-input"]');
  const button=document.querySelector(youtube ? '#send-button button' : '[data-a-target="chat-send-button"]');
  if(!input || !button) return {ok:false,text:'Sign in to this platform in the browser chat window, or check whether this chat allows messages.'};
  if(!text.trim() || [...text].length>(youtube?200:500))return {ok:false,text:youtube?'YouTube messages must be 1–200 characters.':'Twitch messages must be 1–500 characters.'};
  if(input.getAttribute('aria-disabled')==='true' || input.disabled)return {ok:false,text:'This platform has disabled the chat input.'};
  // Do not overwrite text being composed in the platform tab.
  const existing=input.value ?? input.innerText ?? input.textContent;
  if(existing.trim() && existing!==text)return {ok:false,text:'There is an unfinished message in the platform tab. Send or clear it there first.'};
  input.focus();
  if(input.tagName==='TEXTAREA'){
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(input,text);
    input.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText',data:text}));
  }else{
    const selection=getSelection(),range=document.createRange();range.selectNodeContents(input);selection.removeAllRanges();selection.addRange(range);
    // Chromium's editing command goes through the site's editor input handling (including Slate).
    if(!document.execCommand('insertText',false,text))return {ok:false,text:'The platform editor could not accept this message. Use its tab to send.'};
  }
  if(button.disabled || button.getAttribute('aria-disabled')==='true')return {ok:false,text:'The platform is not ready to send. Check its tab for sign-in, slow mode or chat restrictions.'};
  button.click();
  return {ok:true,text:'Submitted to '+(youtube?'YouTube':'Twitch')+'. Delivery is confirmed when it appears in chat.'};
})
