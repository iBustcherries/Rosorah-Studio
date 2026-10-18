import importlib.util,json,unittest,tempfile,zipfile
from pathlib import Path
spec=importlib.util.spec_from_file_location('importer',Path(__file__).with_name('import_external.py'));mod=importlib.util.module_from_spec(spec);spec.loader.exec_module(mod)
def base():return dict(next_id=30,sources=[],scenes=[],rules=[],audio=[],preferences={},canvases=[dict(id=20)])
class ImportTests(unittest.TestCase):
 def test_firebot_disabled_and_scripts_excluded(self):
  with tempfile.TemporaryDirectory() as d:
   p=Path(d)/'events.json';p.write_text(json.dumps({'mainEvents':[{'type':'twitch:follow','effects':{'list':[{'type':'firebot:chat','message':'Thanks $username'},{'type':'firebot:customscript','path':'evil.js'}]}}]}))
   result=mod.run(dict(path=str(p),project=base()));r=result['project']['rules'][0];self.assertFalse(r['enabled']);self.assertEqual(r['actions'][0]['SendChat']['text'],'Thanks {user}');self.assertEqual(len(r['actions']),1);self.assertTrue(any('customscript' in w for w in result['warnings']))
 def test_obs_order_and_missing_files(self):
  with tempfile.TemporaryDirectory() as d:
   p=Path(d)/'scene.json';p.write_text(json.dumps({'sources':[{'id':'image_source','name':'Logo','settings':{'file':'Z:/old/logo.png'}},{'id':'scene','name':'Live','settings':{'items':[{'name':'Logo','pos':{'x':100,'y':200},'bounds':{'x':400,'y':200},'align':5}]}}]}));r=mod.run(dict(path=str(p),project=base()));self.assertEqual(r['project']['scenes'][0]['layers'][0]['rect'],[100/1920,200/1080,400/1920,200/1080]);self.assertEqual(r['missing'],['Z:/old/logo.png'])
 def test_zip_traversal_is_rejected_without_extraction(self):
  with tempfile.TemporaryDirectory() as d:
   p=Path(d)/'bad.zip'
   with zipfile.ZipFile(p,'w') as z:z.writestr('../events.json','{}')
   with self.assertRaises(ValueError):mod.documents(p)
 def test_credentials_are_not_loaded(self):
  with tempfile.TemporaryDirectory() as d:
   p=Path(d)/'backup.zip'
   with zipfile.ZipFile(p,'w') as z:z.writestr('profiles/main/auth.json','{"token":"private"}');z.writestr('profiles/main/events/events.json','{}')
   docs=mod.documents(p);self.assertEqual(len(docs),1);self.assertNotIn('private',str(docs))
if __name__=='__main__':unittest.main()
