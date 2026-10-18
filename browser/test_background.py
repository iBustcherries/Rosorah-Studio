"""Lifecycle unit tests using real companion methods and mocked native widgets."""
import importlib.util
import json
import unittest
import sys
sys.path.insert(0,str(__import__("pathlib").Path(__file__).parent))
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch
spec=importlib.util.spec_from_file_location('chat_browser',Path(__file__).with_name('chat_browser.py'))
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)

class BackgroundTests(unittest.TestCase):
    def test_hidden_start_and_explicit_show(self):
        app=Mock();app.exec.return_value=0
        window=Mock()
        with patch.object(m,'QApplication',return_value=app),patch.object(m,'Window',return_value=window),patch.object(m.sys,'argv',['chat_browser.py','--background']):
            self.assertEqual(m.main(),0)
        app.setQuitOnLastWindowClosed.assert_called_once_with(False)
        window.show.assert_not_called()
        m.Window.command(window,json.dumps({'type':'show'}))
        window.showNormal.assert_called_once();window.activateWindow.assert_called_once()

    def test_closing_window_keeps_timer_and_parent_quit_stops_it(self):
        window=Mock();window.popups=[Mock()];event=Mock()
        with patch.object(m,'emit'):
            m.Window.closeEvent(window,event)
        event.ignore.assert_called_once();window.hide.assert_called_once()
        window.timer.stop.assert_not_called();window.popups[0].hide.assert_called_once()
        with patch.object(m,'QApplication') as app:
            m.Window.shutdown(window)
            window.timer.stop.assert_called_once();app.quit.assert_called_once()
        m.Window.command(window,json.dumps({'type':'quit'}));window.shutdown.assert_called_once()

    def test_hidden_page_collection_and_stale_source_callbacks(self):
        page=Mock();view=Mock();view.page.return_value=page;view.url.return_value.host.return_value='www.twitch.tv'
        window=SimpleNamespace(sources={'Twitch':view},polling=set(),poll_age={},last={'Twitch':'moe'},deliver=Mock())
        window.matches=lambda p,u:True
        window.received=lambda platform,result,ident:m.Window.received(window,platform,result,ident)
        m.Window.poll(window)
        page.setLifecycleState.assert_called_once_with(m.QWebEnginePage.LifecycleState.Active)
        callback=page.runJavaScript.call_args.args[2]
        message={'type':'message','platform':'Twitch','id':'one','text':'Background message'}
        callback(json.dumps([message]))
        window.deliver.assert_called_once_with(message)
        self.assertFalse(window.polling)
        window.last['Twitch']='new_account'
        callback(json.dumps([message]))
        self.assertEqual(window.deliver.call_count,1)

class BridgeTests(unittest.TestCase):
    def test_immediate_delivery_and_stale_source_rejection(self):
        page=Mock();view=Mock();view.page.return_value=page
        window=SimpleNamespace(sources={'Twitch':view},last={'Twitch':'moe'},deliver=Mock())
        window.matches=lambda p,u:m.Window.matches(window,p,u)
        bridge=SimpleNamespace(window=window,platform='Twitch',page=page)
        message=json.dumps({'type':'message','platform':'Twitch','id':'1','text':'hi'})
        m.CollectorBridge.post(bridge,'https://www.twitch.tv/popout/moe/chat?popout=',message)
        self.assertEqual(window.deliver.call_count,1)
        window.last['Twitch']='new'
        m.CollectorBridge.post(bridge,'https://www.twitch.tv/popout/moe/chat?popout=',message)
        m.CollectorBridge.post(bridge,'https://evil.test/popout/new/chat',message)
        self.assertEqual(window.deliver.call_count,1)

    def test_composer_routes_to_shared_account_without_website_login(self):
        window=SimpleNamespace()
        with patch.object(m,'emit') as emit:
            m.Window.post_message(window,'Twitch','hello 👋','2')
            emit.assert_called_once_with({'type':'post_request','platform':'Twitch','text':'hello 👋','request':'2'})
            m.Window.post_message(window,'YouTube','x'*201,'3')
            m.Window.post_message(window,'Other','hello','4')
            self.assertEqual(emit.call_count,1)
        with patch.object(m,'emit') as emit:
            m.ComposerBridge.open(window,'YouTube')
            emit.assert_called_once_with({'type':'open_accounts'})

if __name__=='__main__':unittest.main()
