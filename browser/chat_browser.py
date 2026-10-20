#!/usr/bin/env python3
"""Isolated browser chat. JSON lines only over parent-owned stdin/stdout; no local HTTP port."""
import json, os, re, sys, threading, time
from pathlib import Path
from PySide6.QtCore import QUrl, QTimer, Signal, QObject, Slot, QFile, QIODevice, Qt
from PySide6.QtWebChannel import QWebChannel
from emotes import Images, safe_url
from PySide6.QtWidgets import QApplication, QMainWindow, QTabWidget, QToolBar
from PySide6.QtWebEngineCore import QWebEngineProfile, QWebEnginePage, QWebEngineScript, QWebEngineSettings
from PySide6.QtWebEngineWidgets import QWebEngineView
BASE=Path(__file__).resolve().parent
WORLD=QWebEngineScript.ScriptWorldId.ApplicationWorld.value

def channel_js():
    file=QFile(':/qtwebchannel/qwebchannel.js')
    if not file.open(QIODevice.OpenModeFlag.ReadOnly):raise RuntimeError('Qt WebChannel script is unavailable')
    return bytes(file.readAll()).decode()

class CollectorBridge(QObject):
    def __init__(self,window,platform,page):
        super().__init__(page);self.window=window;self.platform=platform;self.page=page
    @Slot(str,str)
    def post(self,href,payload):
        if len(payload)>65536:return
        view=self.window.sources.get(self.platform)
        if view is None or view.page()!=self.page or not self.window.matches(self.platform,href):return
        try:item=json.loads(payload)
        except ValueError:return
        if isinstance(item,dict) and item.get('platform')==self.platform:self.window.deliver(item)

class ComposerBridge(QObject):
    def __init__(self,window):super().__init__(window);self.window=window
    @Slot(str,str,str)
    def send(self,platform,text,request):self.window.post_message(platform,text,request)
    @Slot(str)
    def open(self,platform):emit({'type':'open_accounts'})


def emit(item):
    try: print(json.dumps(item,ensure_ascii=False),flush=True)
    except (BrokenPipeError,OSError): QApplication.quit()

class Inbox(QObject):
    line=Signal(str)
class Page(QWebEnginePage):
    def __init__(self, *args):
        super().__init__(*args)
        # Collection is an active task even when the companion window is hidden.
        self.recommendedStateChanged.connect(lambda _state: self.setLifecycleState(QWebEnginePage.LifecycleState.Active))
    def javaScriptConsoleMessage(self,*args): pass
    def createWindow(self,kind):
        # Popup stays in the same private profile and requires user navigation.
        view=QWebEngineView(); page=Page(self.profile(),view);view.setPage(page)
        self.window.popups.append(view);page.window=self.window;view.resize(900,700)
        if self.window.isVisible():view.show()
        return page
class ChatView(QWebEngineView):
    def hideEvent(self,event):
        super().hideEvent(event)
        # These pages perform an ongoing foreground task even with the UI tucked away.
        self.page().setVisible(True)
        self.page().setLifecycleState(QWebEnginePage.LifecycleState.Active)
class OffscreenChatView(ChatView):
    # Kick's chat list only renders rows that have layout space; a hidden tab has none, so it
    # stays laid out at a real size without appearing on screen until the user opens it.
    def __init__(self,title):
        super().__init__();self.setWindowTitle(title);self.resize(420,900);self.tuck()
    def tuck(self):
        self.hide();self.setAttribute(Qt.WidgetAttribute.WA_DontShowOnScreen,True);self.show()
    def reveal(self):
        self.hide();self.setAttribute(Qt.WidgetAttribute.WA_DontShowOnScreen,False);self.showNormal();self.raise_();self.activateWindow()
    def closeEvent(self,event):
        event.ignore();self.tuck()

class Window(QMainWindow):
    def __init__(self):
        super().__init__();self.setWindowTitle('Rosorah · Browser chat & activity');self.resize(700,850)
        self.tabs=QTabWidget();self.setCentralWidget(self.tabs);self.sources={};self.popups=[];self.events=[];self.polling=set();self.last={};self.failures={};self.poll_age={}
        path=(Path(os.environ.get('APPDATA',str(Path.home()/'AppData/Roaming'))) if sys.platform=='win32' else Path(os.environ.get('XDG_DATA_HOME',str(Path.home()/'.local/share'))))/'rosorah-studio/browser-profile'
        path.mkdir(parents=True,exist_ok=True);path.chmod(0o700)
        self.profile=QWebEngineProfile('rosorah-chat',self)
        self.profile.setPersistentStoragePath(str(path));self.profile.setCachePath(str(path/'cache'))
        self.profile.setPersistentCookiesPolicy(QWebEngineProfile.PersistentCookiesPolicy.AllowPersistentCookies)
        self.combined=QWebEngineView(); self.combined.settings().setAttribute(QWebEngineSettings.WebAttribute.LocalContentCanAccessRemoteUrls,True); self.tabs.addTab(self.combined,'Combined');self.combined.setHtml((BASE/'dock.html').read_text(),QUrl('about:blank'))
        self.ready=False;self.combined.loadFinished.connect(self.combined_ready)
        self.composer=ComposerBridge(self);self.dock_channel=QWebChannel(self.combined.page());self.dock_channel.registerObject('composer',self.composer);self.combined.page().setWebChannel(self.dock_channel)
        self.images=Images(self,emit)
        self.collector=(BASE/'collector.js').read_text()
        self.webchannel_js=channel_js()
        toolbar=QToolBar();self.addToolBar(toolbar);toolbar.addAction('Reconnect chats',self.reload)
        self.statusBar().showMessage('Chat runs in the background. Closing this window keeps collection active; closing Rosorah stops it.')
        self.timer=QTimer(self);self.timer.timeout.connect(self.poll);self.timer.start(5000)
        self.inbox=Inbox();self.inbox.line.connect(self.command)
        threading.Thread(target=self.read_commands,daemon=True).start()
        emit({'type':'status','text':'Chat is running in the background. Messages are delivered immediately as the platform page receives them.'})
    def combined_ready(self,ok):
        self.ready=ok
        if ok:
            self.combined.page().runJavaScript(self.webchannel_js+'\nnew QWebChannel(qt.webChannelTransport,c=>window.connectComposer(c.objects.composer));')
            self.update_targets()
            for item in self.events:self.combined.page().runJavaScript('window.ingest('+json.dumps(item)+')')
    def read_commands(self):
        for line in sys.stdin:
            if len(line)<131072:self.inbox.line.emit(line)
        self.inbox.line.emit('{"type":"quit"}')
    def command(self,line):
        try: msg=json.loads(line)
        except ValueError:return
        typ=msg.get('type')
        if typ=='quit':self.shutdown()
        elif typ=='show':
            self.showNormal();self.raise_();self.activateWindow()
        elif typ=='reload':self.reload()
        elif typ=='post_result':
            if self.ready:self.combined.page().runJavaScript('window.postResult('+json.dumps(msg)+')')
        elif typ=='type_message':self.type_message(str(msg.get('text','')),str(msg.get('request',''))[:100])
        elif typ=='diagnose':self.diagnose(str(msg.get('platform','')))
        elif typ=='open_platform':self.open_platform(msg.get('platform',''),bool(msg.get('picker',False)))
        elif typ=='event':self.deliver(msg['event'],forward=False)
        elif typ=='images':
            # Pictures for the emote picker; asking again reloads one that Rosorah let go of.
            for src in msg.get('urls',[])[:64] if isinstance(msg.get('urls'),list) else []:
                if isinstance(src,str) and safe_url(src):self.images.seen.pop(src,None);self.images.request(src)
        elif typ=='sources':
            for platform,ident in [('Twitch',msg.get('twitch','')),('YouTube',msg.get('youtube','')),('Kick',msg.get('kick','')),('TikTok',msg.get('tiktok',''))]:
                pattern={'Twitch':r'[A-Za-z0-9_]{1,25}','YouTube':r'[A-Za-z0-9_-]{11}','Kick':r'[A-Za-z0-9_-]{1,50}','TikTok':r'[A-Za-z0-9_.]{2,24}'}[platform]
                valid=isinstance(ident,str) and bool(re.fullmatch(pattern,ident))
                if not valid:
                    if platform in self.sources:
                        view=self.sources.pop(platform);self.tabs.removeTab(self.tabs.indexOf(view));view.deleteLater();self.last.pop(platform,None);self.polling.discard(platform);self.poll_age.pop(platform,None)
                    continue
                if self.last.get(platform)==ident:continue
                self.last[platform]=ident;self.polling.discard(platform);self.poll_age.pop(platform,None)
                if platform not in self.sources:
                    view=OffscreenChatView('Rosorah · '+platform+' chat') if platform in ('Kick','TikTok') else ChatView();page=Page(self.profile,view);page.window=self;view.setPage(page);self.sources[platform]=view
                    if not isinstance(view,OffscreenChatView):self.tabs.addTab(view,platform)
                    self.install_collector(platform,page)
                    view.loadFinished.connect(lambda ok,p=platform:self.loaded(p,ok))
                    page.renderProcessTerminated.connect(lambda *_args,p=platform:self.failed(p))
                url={'Twitch':'https://www.twitch.tv/popout/'+ident+'/chat?popout=','YouTube':'https://www.youtube.com/live_chat?v='+ident+'&is_popout=1','Kick':'https://kick.com/popout/'+ident+'/chat','TikTok':'https://www.tiktok.com/@'+ident+'/live'}[platform]
                self.sources[platform].setUrl(QUrl(url))
                self.sources[platform].page().setVisible(True)
            self.update_targets()
    def update_targets(self):
        if self.ready:self.combined.page().runJavaScript('window.setAccounts('+json.dumps(list(self.last))+')')
    def matches(self,platform,href):
        url=QUrl(href)
        ident=self.last.get(platform)
        if not ident or url.scheme()!='https':return False
        if platform=='Twitch':return url.host()=='www.twitch.tv' and url.path()==f'/popout/{ident}/chat'
        if platform=='Kick':return url.host() in ('kick.com','www.kick.com') and url.path().rstrip('/').lower()==f'/popout/{ident}/chat'.lower()
        if platform=='TikTok':return url.host() in ('www.tiktok.com','tiktok.com') and url.path().rstrip('/').lower()==f'/@{ident}/live'.lower()
        from PySide6.QtCore import QUrlQuery
        return url.host()=='www.youtube.com' and url.path()=='/live_chat' and QUrlQuery(url).queryItemValue('v')==ident
    def install_collector(self,platform,page):
        page.bridge=CollectorBridge(self,platform,page);page.channel=QWebChannel(page);page.channel.registerObject('collector',page.bridge);page.setWebChannel(page.channel,WORLD)
        script=QWebEngineScript();script.setName('rosorah-live-chat');script.setWorldId(WORLD);script.setInjectionPoint(QWebEngineScript.InjectionPoint.DocumentReady);script.setRunsOnSubFrames(False)
        script.setSourceCode(self.webchannel_js+'\n'+self.collector+"\nnew QWebChannel(qt.webChannelTransport,c=>window.__rosorah.setSink(item=>c.objects.collector.post(location.href,JSON.stringify(item))));")
        page.scripts().insert(script)
    def open_platform(self,platform,picker=False):
        view=self.sources.get(platform)
        if isinstance(view,OffscreenChatView):view.reveal()
        else:
            if view:self.tabs.setCurrentWidget(view)
            self.showNormal();self.raise_();self.activateWindow()
        if picker and platform in self.sources:
            selector={'Twitch':'[data-a-target="emote-picker-button"]','YouTube':'#emoji-picker-button button, #emoji-picker-button','Kick':'[data-testid="emote-picker-button"], button[aria-label*="emote" i]'}.get(platform,'#emoji-picker-button')
            self.sources[platform].page().runJavaScript('document.querySelector('+json.dumps(selector)+')?.click()',WORLD)
    def post_message(self,platform,text,request):
        if platform not in ('Twitch','YouTube','Kick','TikTok') or not isinstance(text,str) or not isinstance(request,str) or len(request)>100:return
        if not text.strip() or len(text)>{'YouTube':200,'TikTok':150}.get(platform,500):return
        # Only explicit composer actions use this bridge. Tokens never enter the browser.
        emit({'type':'post_request','platform':platform,'text':text,'request':request})
    def type_message(self,text,request):
        # TikTok has no chat API: with the user's opt-in, type into their own LIVE page's chat box.
        view=self.sources.get('TikTok')
        def result(ok,detail):emit({'type':'post_result','platform':'TikTok','request':request,'ok':ok,'text':detail})
        if not view or not self.matches('TikTok',view.url().toString()):return result(False,'Your TikTok LIVE page is not open. Check your TikTok username in Settings > Connections.')
        if not text.strip() or len(text)>150:return result(False,'TikTok messages must be 1-150 characters.')
        script='''(()=>{const field=document.querySelector('[data-e2e="room-chat-input-field"]');
 const box=(field&&(field.matches('[contenteditable],textarea')?field:field.querySelector('[contenteditable="true"],[contenteditable="plaintext-only"],textarea')))||[...document.querySelectorAll('[contenteditable="true"],[contenteditable="plaintext-only"],textarea')].filter(e=>e.offsetParent!==null).pop();
 if(!box)return document.querySelector('[data-e2e*="login"],[data-e2e="top-login-button"]')?'login':'no-input';
 box.focus();if(box.tagName==='TEXTAREA'){box.value=__ROSORAH_TEXT__;box.dispatchEvent(new Event('input',{bubbles:true}));}else{document.execCommand('selectAll');document.execCommand('insertText',false,__ROSORAH_TEXT__);}
 const send=document.querySelector('[data-e2e="room-chat-send-btn"],[data-e2e="comment-post"],[data-e2e*="send"],button[aria-label*="send" i],[aria-label*="post" i][role="button"]');
 if(send&&!send.disabled)send.click();else for(const t of ['keydown','keypress','keyup'])box.dispatchEvent(new KeyboardEvent(t,{key:'Enter',code:'Enter',keyCode:13,which:13,bubbles:true}));
 window.__rosorahTyped=box;return 'typed';})()'''.replace('__ROSORAH_TEXT__',json.dumps(text))
        check="(()=>{const b=window.__rosorahTyped;return !b?'gone':((b.value??b.textContent)||'').trim()?'kept':'cleared';})()"
        def typed(state):
            if state=='login':return result(False,'Sign in to TikTok in the chat browser (Settings > Connections > TikTok > Open TikTok page), then try again.')
            if state!='typed':return result(False,'TikTok chat box not found. Make sure you are live and signed in; your draft was kept.')
            QTimer.singleShot(1500,lambda:view.page().runJavaScript(check,WORLD,lambda s:result(True,'Sent') if s in ('cleared','gone') else result(False,"TikTok didn't accept the message; your draft was kept.")))
        view.page().runJavaScript(script,WORLD,typed)
    def diagnose(self,platform):
        # Structure only (tags, data-e2e hooks, counts): no message text or names, so it is safe to share.
        view=self.sources.get(platform)
        if not view:return emit({'type':'diagnostic','platform':platform,'text':platform+' page is not open.'})
        script='''(()=>{const hooks={};document.querySelectorAll('[data-e2e]').forEach(e=>{const k=e.getAttribute('data-e2e');hooks[k]=(hooks[k]||0)+1;});
 const shape=(e,d)=>d>5||!e?null:{tag:e.tagName.toLowerCase(),e2e:e.getAttribute('data-e2e')||undefined,text:e.childNodes.length&&[...e.childNodes].some(n=>n.nodeType===3&&n.textContent.trim())?'…':undefined,kids:[...e.children].slice(0,6).map(c=>shape(c,d+1)).filter(Boolean)};
 const rows=[...document.querySelectorAll('[data-e2e$="message"]')].slice(-3).map(e=>shape(e,0));
 let list=document.querySelector('[data-e2e="live-chat-container"]');while(list&&list.children.length===1)list=list.children[0];
 const chat=list?[...list.querySelectorAll('[class]')].filter(e=>e.children.length>3).slice(-1).flatMap(e=>[...e.children].slice(-3)).map(e=>shape(e,0)):[];
 return JSON.stringify({url:location.pathname,title:document.title.slice(0,80),hooks,rows,chat,editors:document.querySelectorAll('[contenteditable]').length},null,1);})()'''
        view.page().runJavaScript(script,WORLD,lambda r:emit({'type':'diagnostic','platform':platform,'text':(r or 'No result')[:60000]}))
    def loaded(self,platform,ok):
        self.polling.discard(platform)
        self.failures[platform]=0 if ok else min(self.failures.get(platform,0)+1,5)
        if not ok: QTimer.singleShot(min(30000,1000*2**self.failures[platform]),lambda:self.sources.get(platform) and self.sources[platform].reload())
        emit({'type':'status','text':platform+(' chat page loaded; waiting for messages' if ok else ' chat page failed; retrying automatically.')})
    def failed(self,platform):
        self.polling.discard(platform);emit({'type':'status','text':platform+' browser renderer stopped; reopening in 5 seconds'})
        QTimer.singleShot(5000,lambda:self.sources.get(platform) and self.sources[platform].reload())
    def reload(self):
        self.polling.clear()
        for view in self.sources.values():view.reload()
        emit({'type':'status','text':'Reopening browser chats…'})
    def poll(self):
        script='if(window.__rosorah)window.__rosorah.scan();'
        for p,v in self.sources.items():
            if not self.matches(p,v.url().toString()):continue
            if p in self.polling:
                if time.monotonic()-self.poll_age.get(p,time.monotonic())>20:
                    self.polling.discard(p);self.failed(p)
                continue
            v.page().setLifecycleState(QWebEnginePage.LifecycleState.Active)
            self.polling.add(p);self.poll_age[p]=time.monotonic()
            ident=self.last.get(p)
            v.page().runJavaScript(script+'\nJSON.stringify(window.__rosorah ? window.__rosorah.drain() : []);',WORLD,lambda result,p=p,ident=ident:self.received(p,result,ident))
    def received(self,platform,result,ident):
        if self.last.get(platform)!=ident:return
        self.polling.discard(platform);self.poll_age.pop(platform,None)
        try: items=json.loads(result or '[]')
        except (ValueError,TypeError):return
        if isinstance(items,list):
            for item in items[:500]:self.deliver(item)
    def deliver(self,item,forward=True):
        if forward and isinstance(item,dict) and item.get('type')=='emote_list' and item.get('platform')=='YouTube':
            items=[{'name':str(e.get('name',''))[:100],'src':e['src']} for e in item.get('items',[])[:300] if isinstance(e,dict) and isinstance(e.get('src'),str) and safe_url(e['src']) and e.get('name')]
            emit({'type':'emote_list','platform':'YouTube','items':items});return
        if not isinstance(item,dict) or item.get('type') not in ('message','delete'):return
        if forward:
            item["source"]=self.last.get(item.get("platform"), "")
            clean=[]
            for part in item.get('parts',[])[:128]:
                if not isinstance(part,dict):continue
                entry={'text':str(part.get('text',''))[:5000]}
                if safe_url(part.get('src','')):entry['src']=part['src'];self.images.request(part['src'])
                clean.append(entry)
            item['parts']=clean
            photo=item.get('author_photo');item['author_photo']=photo[:2048] if isinstance(photo,str) and photo.startswith('https://') else ''
        self.events.append(item);self.events=self.events[-500:]
        if self.ready:self.combined.page().runJavaScript('window.ingest('+json.dumps(item)+')')
        if forward:emit(item)
    def closeEvent(self,event):
        event.ignore()
        self.hide()
        for popup in self.popups:popup.hide()
        emit({'type':'status','text':'Chat is running in the background. Closing the browser window does not stop collection.'})
    def shutdown(self):
        self.timer.stop()
        QApplication.quit()

def main():
    flags=os.environ.get('QTWEBENGINE_CHROMIUM_FLAGS','')
    for flag in ['--disable-background-timer-throttling','--disable-renderer-backgrounding']:
        if flag not in flags:flags+=' '+flag
    os.environ['QTWEBENGINE_CHROMIUM_FLAGS']=flags.strip()
    app=QApplication(sys.argv)
    app.setQuitOnLastWindowClosed(False)
    window=Window()
    if '--background' not in sys.argv:window.show()
    return app.exec()

if __name__=='__main__':sys.exit(main())
