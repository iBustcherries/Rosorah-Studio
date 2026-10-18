import importlib.util, json, tempfile, zipfile
from pathlib import Path
module=Path(__file__).resolve().parents[1]/'packaging/project_bundle.py'
spec=importlib.util.spec_from_file_location('bundle',module);bundle=importlib.util.module_from_spec(spec);spec.loader.exec_module(bundle)
with tempfile.TemporaryDirectory() as tmp:
    root=Path(tmp)
    for name in ['../escape','/tmp/escape','media/../../escape','media/../escape','media\\escape']:
        path=root/'unsafe.zip'
        with zipfile.ZipFile(path,'w') as z:
            z.writestr('project.json',json.dumps({'sources':[]}));z.writestr(name,'unsafe')
        try:bundle.run({'mode':'import','path':str(path),'parent':str(root)})
        except ValueError:pass
        else:raise AssertionError(name)
        assert not list(root.glob('Rosorah-import-*'))
    with zipfile.ZipFile(root/'unsafe.zip','w') as z:
        z.writestr('project.json',json.dumps({'sources':[{'kind':{'Image':{'path':'/etc/passwd'}}}]}))
    try:bundle.run({'mode':'import','path':str(root/'unsafe.zip'),'parent':str(root)})
    except ValueError:pass
    else:raise AssertionError('external media accepted')
print('PASS: path traversal, absolute paths, backslashes and external media references rejected; partial imports cleaned')
