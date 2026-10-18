import asyncio, base64, json, tempfile, unittest
from pathlib import Path
from aiohttp.test_utils import TestClient, TestServer
from server import Store, make_app

class ReceiverTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.path=str(Path(self.tmp.name)/'relay.db')
        store=Store(self.path)
        def create():
            code=store.create('https://relay.example');return json.loads(base64.urlsafe_b64decode(code+'='*(-len(code)%4)))['key']
        self.key=create();self.other=create();store.db.close()
        self.client=TestClient(TestServer(make_app(self.path,'https://relay.example',wait_seconds=.05)))
        await self.client.start_server();self.headers={'Authorization':'Bearer '+self.key}
        r=await self.client.post('/v1/configure',headers=self.headers,json={'verification_token':'kofi-secret-for-tests'})
        self.status=await r.json();self.hook='/hooks/kofi/'+self.status['owner'].split(':')[1]
    async def asyncTearDown(self):
        await self.client.close();self.tmp.cleanup()
    async def post(self,**updates):
        data={'verification_token':'kofi-secret-for-tests','message_id':'m1','type':'Donation','is_public':True,'from_name':'Viewer','message':'hello','amount':'12.34','currency':'USD','email':'never-store@example.com','shipping':{'address':'private'}}
        data.update(updates)
        return await self.client.post(self.hook,data={'data':json.dumps(data)})
    async def enable(self):
        self.assertEqual((await self.post()).status,200)
        self.assertEqual((await self.client.post('/v1/live',headers=self.headers,json={'live':True})).status,200)
    async def batch(self):return await (await self.client.get('/v1/events',headers=self.headers)).json()
    async def test_setup_test_dedup_private_fields_and_live_goal_amount(self):
        self.assertEqual((await self.client.post('/v1/live',headers=self.headers,json={'live':True})).status,409)
        await self.enable()
        self.assertEqual((await self.batch())['events'],[])
        await self.post() # retry setup test remains ignored after going live
        self.assertEqual((await self.batch())['events'],[])
        await self.post(message_id='real',is_public=False)
        await self.post(message_id='real',is_public=False)
        data=await self.batch();self.assertEqual(len(data['events']),1)
        event=data['events'][0];self.assertEqual(event['hundredths'],1234)
        self.assertEqual(event['name'],'Anonymous');self.assertEqual(event['message'],'')
        self.assertNotIn('email',json.dumps(event));self.assertNotIn('shipping',json.dumps(event))
        r=await self.client.post('/v1/ack',headers=self.headers,json={'seq':event['seq']});self.assertEqual(r.status,200)
        self.assertEqual((await self.batch())['events'],[])
    async def test_auth_isolation_and_rejected_input(self):
        self.assertEqual((await self.post(verification_token='wrong')).status,401)
        self.assertEqual((await self.client.get('/v1/events')).status,401)
        await self.enable();await self.post(message_id='a')
        other={'Authorization':'Bearer '+self.other}
        data=await (await self.client.get('/v1/events',headers=other)).json();self.assertEqual(data['events'],[])
        self.assertEqual((await self.client.post('/v1/ack',headers=other,json={'seq':1})).status,400)
        for value in ['NaN','Infinity','-1','0','1.001','1e999999']:
            self.assertEqual((await self.post(message_id='bad-'+value,amount=value)).status,400)
        self.assertEqual((await self.client.post(self.hook,data={'data':'x'*70000})).status,413)
    async def test_reconnect_replays_until_ack_and_revocation(self):
        await self.enable();await self.post(message_id='saved')
        a=await self.batch();b=await self.batch();self.assertEqual(a['events'],b['events'])
        self.assertEqual((await self.client.delete('/v1/account',headers=self.headers)).status,200)
        self.assertEqual((await self.client.get('/v1/status',headers=self.headers)).status,401)
        self.assertEqual((await self.post(message_id='after')).status,401)
    async def test_notification_wakes_waiter(self):
        await self.enable()
        waiting=asyncio.create_task(self.client.get('/v1/events',headers=self.headers))
        await asyncio.sleep(.01);await self.post(message_id='push')
        data=await (await waiting).json();self.assertEqual(data['events'][0]['id'],'push')
    async def test_restart_retains_events_and_bounded_queue_retries(self):
        from unittest.mock import patch
        await self.enable()
        with patch('server.MAX_EVENTS',0):
            self.assertEqual((await self.post(message_id='retry')).status,503)
        self.assertEqual((await self.post(message_id='retry')).status,200)
        await self.client.close()
        self.client=TestClient(TestServer(make_app(self.path,'https://relay.example',wait_seconds=.01)))
        await self.client.start_server()
        self.assertEqual((await self.batch())['events'][0]['id'],'retry')
        with patch('server.MAX_SEEN',0):
            self.assertEqual((await self.post(message_id='quota')).status,503)
        self.assertEqual((await self.post(message_id='quota')).status,200)

    async def test_pause_and_unsupported_payments_do_not_count(self):
        await self.enable();await self.post(message_id='shop',type='Shop Order')
        await self.post(message_id='flagged',is_test=True)
        self.assertEqual((await self.batch())['events'],[])
        await self.client.post('/v1/live',headers=self.headers,json={'live':False})
        await self.post(message_id='test2')
        self.assertEqual((await self.batch())['events'],[])

if __name__=='__main__':unittest.main()
