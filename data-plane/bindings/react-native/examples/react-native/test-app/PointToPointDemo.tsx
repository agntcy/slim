import React, {useState, useEffect, useRef} from 'react';
import {
  View,
  Text,
  TextInput,
  TouchableOpacity,
  ScrollView,
  StyleSheet,
  useColorScheme,
  Alert,
} from 'react-native';

interface Message {
  id: string;
  timestamp: Date;
  type: 'sent' | 'received' | 'system';
  text: string;
}

interface PointToPointDemoProps {
  slimBindings: any;
}

export default function PointToPointDemo({slimBindings}: PointToPointDemoProps) {
  const isDarkMode = useColorScheme() === 'dark';
  
  // Track if SLIM has been initialized
  const [slimInitialized, setSlimInitialized] = useState(false);
  
  // Track global connection (shared between Alice and Bob)
  const globalConnectionRef = useRef<{connId: bigint, service: any} | null>(null);
  
  // State
  const [serverUrl, setServerUrl] = useState('http://localhost:46357');
  const [message, setMessage] = useState('Hello from React Native!');
  const [messages, setMessages] = useState<Message[]>([]);
  const [status, setStatus] = useState<'idle' | 'alice' | 'bob'>('idle');
  const [isRunning, setIsRunning] = useState(false);
  
  // Refs to store app/session state
  const appRef = useRef<any>(null);
  const sessionRef = useRef<any>(null);
  const runningRef = useRef(false);
  
  // Cleanup on unmount
  useEffect(() => {
    return () => {
      runningRef.current = false;
      if (sessionRef.current && appRef.current) {
        try {
          appRef.current.deleteSessionAndWaitAsync(sessionRef.current);
        } catch (e) {
          console.log('Cleanup error:', e);
        }
      }
      // Clear global connection on unmount
      console.log('[Cleanup] Clearing global connection ref');
      globalConnectionRef.current = null;
    };
  }, []);
  
  const addMessage = (type: 'sent' | 'received' | 'system', text: string) => {
    const msg: Message = {
      id: Date.now().toString() + Math.random(),
      timestamp: new Date(),
      type,
      text,
    };
    setMessages(prev => [...prev, msg]);
  };
  
  const parseName = (id: string) => {
    const parts = id.split('/');
    if (parts.length !== 3) {
      throw new Error(`IDs must be in format org/namespace/app, got: ${id}`);
    }
    return new slimBindings.Name(parts[0], parts[1], parts[2]);
  };
  
  const createAndConnectApp = async (localId: string) => {
    console.log('[createAndConnectApp] Starting for:', localId);
    
    try {
      // Only initialize once per app lifecycle
      if (!slimInitialized) {
        addMessage('system', '🚀 Initializing SLIM...');
        console.log('[createAndConnectApp] Calling initializeWithDefaults');
        slimBindings.initializeWithDefaults();
        setSlimInitialized(true);
        console.log('[createAndConnectApp] Initialized');
      } else {
        console.log('[createAndConnectApp] Already initialized, skipping');
      }
    } catch (e: any) {
      console.error('[createAndConnectApp] Initialize error:', e);
      throw new Error(`Initialize failed: ${e.message}`);
    }
    
    try {
      const appName = parseName(localId);
      console.log('[createAndConnectApp] Parsed name:', appName);
      
      const service = slimBindings.getGlobalService();
      console.log('[createAndConnectApp] Got service');
      
      const app = service.createAppWithSecret(
        appName,
        'demo-shared-secret-min-32-chars!!'
      );
      console.log('[createAndConnectApp] Created app, ID:', app.id());
      
      addMessage('system', `✅ Created app: ${localId}`);
      
      // Check if we already have a connection (shared between Alice and Bob)
      let connId: bigint;
      if (globalConnectionRef.current) {
        console.log('[createAndConnectApp] Reusing existing connection:', globalConnectionRef.current.connId);
        connId = globalConnectionRef.current.connId;
        addMessage('system', `🔗 Reusing connection (ID: ${connId})`);
      } else {
        addMessage('system', `🔌 Connecting to ${serverUrl}...`);
        console.log('[createAndConnectApp] Connecting to:', serverUrl);
        
        const config = slimBindings.newInsecureClientConfig(serverUrl);
        console.log('[createAndConnectApp] Created config');
        
        try {
          connId =  service.connect(config);
        } catch (e: any) {
          console.error('[createAndConnectApp] connectAsync error:',e.inner);
          console.error('[createAndConnectApp] connectAsync error stack:', e.stack);
          addMessage('system', `❌ connectAsync failed: ${e.message || String(e)}`);
          throw new Error(`connectAsync failed: ${e.message || String(e)}`);
        }

        
        // Store for reuse
        globalConnectionRef.current = { connId, service };
        addMessage('system', `✅ Connected (conn ID: ${connId})`);
      }
      
      console.log('[createAndConnectApp] Subscribing...');
      await app.subscribeAsync(appName, connId);
      console.log('[createAndConnectApp] Subscribed');
      
      addMessage('system', '✅ Subscribed to sessions');
      
      return {app, connId, service};
    } catch (e: any) {
      console.error('[createAndConnectApp] Error:', e);
      console.error('[createAndConnectApp] Error type:', e.constructor?.name);
      throw new Error(`App creation/connection failed: ${e.message || String(e)}`);
    }
  };
  
  const startAlice = async () => {
    if (isRunning) return;
    
    setIsRunning(true);
    setStatus('alice');
    setMessages([]);
    runningRef.current = true;
    
    try {
      const {app} = await createAndConnectApp('org/alice/app');
      appRef.current = app;
      
      addMessage('system', '👂 Waiting for incoming sessions...');
      
      // Listen for sessions
      while (runningRef.current) {
        try {
          const session = await app.listenForSessionAsync(undefined);
          sessionRef.current = session;
          addMessage('system', '🎉 New session established!');
          
          // Handle session messages
          await handleAliceSession(app, session);
        } catch (err: any) {
          if (runningRef.current) {
            addMessage('system', `⏱️ Session timeout: ${err.message}`);
          }
        }
      }
    } catch (err: any) {
      addMessage('system', `❌ Error: ${err.message}`);
      Alert.alert('Error', err.message);
    } finally {
      setIsRunning(false);
      setStatus('idle');
    }
  };
  
  const handleAliceSession = async (app: any, session: any) => {
    console.log('[Alice] Handling new session');
    try {
      while (runningRef.current) {
        console.log('[Alice] Waiting for message...');
        const msg = await session.getMessageAsync(60000);
        console.log('[Alice] Received message, payload type:', typeof msg.payload);
        console.log('[Alice] Received message, payload constructor:', msg.payload?.constructor?.name);
        
        // Convert payload to string
        let text: string;
        if (typeof msg.payload === 'string') {
          text = msg.payload;
        } else if (msg.payload instanceof ArrayBuffer) {
          // Convert ArrayBuffer to Uint8Array, then to string
          const bytes = new Uint8Array(msg.payload);
          text = String.fromCharCode(...bytes);
        } else if (msg.payload instanceof Uint8Array || Array.isArray(msg.payload)) {
          // Convert byte array to string
          text = String.fromCharCode(...(msg.payload as Uint8Array));
        } else {
          text = String(msg.payload);
        }
        
        console.log('[Alice] Decoded text:', text);
        addMessage('received', text);
        
        // Echo reply
        const reply = `${text} (echoed by Alice)`;
        // Convert string to byte array
        const replyPayload = new Uint8Array(reply.split('').map(c => c.charCodeAt(0)));
        console.log('[Alice] Sending reply:', reply);
        
        await session.publishToAndWaitAsync(msg.context, replyPayload, undefined, undefined);
        console.log('[Alice] Reply sent');
        
        addMessage('sent', reply);
      }
    } catch (err: any) {
      console.log('[Alice] Session ended:', err);
      // Check if it's just a normal session closure (expected)
      const isNormalClosure = err.inner?.message === 'session closed' || 
                              err.message === 'session closed' ||
                              err.tag === 'SessionError';
      
      if (isNormalClosure) {
        addMessage('system', '🔚 Session ended normally');
      } else {
        console.error('[Alice] Unexpected session error:', err);
        addMessage('system', `❌ Session error: ${err.message || String(err)}`);
      }
    } finally {
      try {
        console.log('[Alice] Cleaning up session');
        await app.deleteSessionAndWaitAsync(session);
        addMessage('system', '👋 Session closed');
      } catch (e: any) {
        // Silently ignore "already closed" errors - it's expected when peer closes first
        if (!e.inner?.message?.includes('already closed')) {
          console.log('[Alice] Session cleanup error:', e);
        }
      }
    }
  };
  
  const startBob = async () => {
    if (isRunning) return;
    
    setIsRunning(true);
    setStatus('bob');
    setMessages([]);
    runningRef.current = true;
    
    console.log('[Bob] === Starting Bob ===');
    
    try {
      addMessage('system', '🚀 Starting Bob...');
      console.log('[Bob] Step 1: Creating and connecting app');
      const {app, connId} = await createAndConnectApp('org/bob/app');
      console.log('[Bob] Step 1 complete. ConnId type:', typeof connId, 'value:', connId);
      appRef.current = app;
      
      console.log('[Bob] Step 2: Parsing remote name');
      addMessage('system', '🔍 Parsing remote name...');
      const remoteName = parseName('org/alice/app');
      console.log('[Bob] Step 2 complete. RemoteName:', remoteName);
      
      console.log('[Bob] Step 3: Setting route. ConnId:', connId, 'Type:', typeof connId);
      addMessage('system', `📍 Setting route to Alice (connId: ${connId})...`);
      try {
        // Ensure connId is the right type - try both number and bigint
        const connIdForRoute = typeof connId === 'bigint' ? connId : BigInt(connId);
        console.log('[Bob] Calling setRoute with connId:', connIdForRoute, 'type:', typeof connIdForRoute);
        await app.setRoute(remoteName, connIdForRoute);
        console.log('[Bob] Step 3 complete');
        addMessage('system', '✅ Route set');
        
        // Give route time to propagate (like Go example)
        console.log('[Bob] Waiting 100ms for route to propagate...');
        await new Promise(resolve => setTimeout(resolve, 100));
      } catch (e: any) {
        console.error('[Bob] setRoute error:', e);
        console.error('[Bob] setRoute error stack:', e.stack);
        addMessage('system', `❌ setRoute failed: ${e.message || String(e)}`);
        throw new Error(`setRoute failed: ${e.message || String(e)}`);
      }
      
      console.log('[Bob] Step 4: Creating session config');
      // metadata MUST be a Map object (required field)
      const config: any = {
        sessionType: slimBindings.SessionType.PointToPoint,
        enableMls: false,
        metadata: new Map(),  // Required: Map object
      };
      console.log('[Bob] Config created with metadata Map, size:', config.metadata.size);
      
      console.log('[Bob] Step 5: Creating session');
      addMessage('system', '🔍 Creating session to Alice...');
      try {
        // Use createSessionAndWaitAsync (matches Go's CreateSessionAndWait)
        console.log('[Bob] Calling createSessionAndWaitAsync...');
        let session: any;

        try {
          session = await app.createSessionAndWaitAsync(config, remoteName);
          sessionRef.current = session;
          addMessage('system', '📡 Session created');
        } catch (e: any) {
          console.error('[Bob] createSessionAndWaitAsync error:', e);
          console.error('[Bob] createSessionAndWaitAsync error:', e.inner);
          console.error('[Bob] createSessionAndWaitAsync error stack:', e.stack);
          addMessage('system', `❌ createSessionAndWaitAsync failed: ${e.inner || String(e.inner)}`);
          throw new Error(`createSessionAndWaitAsync failed: ${e.inner || String(e.inner)}`);
        }
        
        console.log('[Bob] Session created and ready');
        sessionRef.current = session;
        addMessage('system', '📡 Session created');
        
        console.log('[Bob] Step 6: Sending messages');
        // Send messages
        for (let i = 0; i < 3 && runningRef.current; i++) {
          console.log(`[Bob] Sending message ${i + 1}/3`);
          const msgText = `${message} #${i + 1}`;
          // Convert string to byte array (no TextEncoder in React Native)
          const payload = new Uint8Array(msgText.split('').map(c => c.charCodeAt(0)));
          await session.publishAndWaitAsync(payload, undefined, undefined);
          addMessage('sent', msgText);
          
          // Wait for reply
          try {
            const replyMsg = await session.getMessageAsync(5000);
            // Convert payload to string (handle ArrayBuffer from React Native)
            let replyText: string;
            if (typeof replyMsg.payload === 'string') {
              replyText = replyMsg.payload;
            } else if (replyMsg.payload instanceof ArrayBuffer) {
              const bytes = new Uint8Array(replyMsg.payload);
              replyText = String.fromCharCode(...bytes);
            } else if (replyMsg.payload instanceof Uint8Array || Array.isArray(replyMsg.payload)) {
              replyText = String.fromCharCode(...(replyMsg.payload as Uint8Array));
            } else {
              replyText = String(replyMsg.payload);
            }
            addMessage('received', replyText);
            console.log(`[Bob] Received reply ${i + 1}:`, replyText);
          } catch (e: any) {
            console.log(`[Bob] No reply for message ${i + 1}`);
            addMessage('system', `⏱️ No reply: ${e.message || 'timeout'}`);
          }
          
          // Wait between messages
          await new Promise(resolve => setTimeout(resolve, 1000));
        }
        
        console.log('[Bob] Step 7: Cleanup');
        addMessage('system', '✅ Sending complete');
        
        // Cleanup
        await app.deleteSessionAndWaitAsync(session);
        addMessage('system', '👋 Session closed');
        console.log('[Bob] === Bob complete ===');
      } catch (e: any) {
        console.error('[Bob] Session error:', e);
        console.error('[Bob] Session error stack:', e.stack);
        addMessage('system', `❌ Session error: ${e.message || String(e)}`);
        throw new Error(`Session failed: ${e.message || String(e)}`);
      }
      
    } catch (err: any) {
      const errorMsg = err.message || String(err);
      console.error('[Bob] Fatal error:', err);
      console.error('[Bob] Fatal error stack:', err.stack);
      console.error('[Bob] Error type:', err.constructor?.name);
      console.error('[Bob] Error details:', JSON.stringify(err, null, 2));
      addMessage('system', `❌ Error: ${errorMsg}`);
      Alert.alert('Bob Error', errorMsg);
    } finally {
      console.log('[Bob] Finally block');
      runningRef.current = false;
      setIsRunning(false);
      setStatus('idle');
    }
  };
  
  const stop = () => {
    runningRef.current = false;
    addMessage('system', '🛑 Stopping...');
    setIsRunning(false);
    setStatus('idle');
  };
  
  return (
    <View style={[styles.container, {backgroundColor: isDarkMode ? '#111' : '#f5f5f5'}]}>
      <Text style={[styles.title, {color: isDarkMode ? '#fff' : '#000'}]}>
        Point-to-Point Demo
      </Text>
      
      {/* Server URL Input */}
      <View style={styles.inputContainer}>
        <Text style={[styles.label, {color: isDarkMode ? '#ccc' : '#666'}]}>
          Server URL:
        </Text>
        <TextInput
          style={[
            styles.input,
            {
              backgroundColor: isDarkMode ? '#333' : '#fff',
              color: isDarkMode ? '#fff' : '#000',
            },
          ]}
          value={serverUrl}
          onChangeText={setServerUrl}
          placeholder="http://localhost:46357"
          placeholderTextColor={isDarkMode ? '#888' : '#aaa'}
          editable={!isRunning}
          autoCapitalize="none"
        />
      </View>
      
      {/* Message Input (for Bob) */}
      <View style={styles.inputContainer}>
        <Text style={[styles.label, {color: isDarkMode ? '#ccc' : '#666'}]}>
          Message to Send (Bob):
        </Text>
        <TextInput
          style={[
            styles.input,
            {
              backgroundColor: isDarkMode ? '#333' : '#fff',
              color: isDarkMode ? '#fff' : '#000',
            },
          ]}
          value={message}
          onChangeText={setMessage}
          placeholder="Hello from React Native!"
          placeholderTextColor={isDarkMode ? '#888' : '#aaa'}
          editable={!isRunning}
        />
      </View>
      
      {/* Status */}
      <View style={styles.statusContainer}>
        <Text style={[styles.status, {color: isDarkMode ? '#fff' : '#000'}]}>
          Status: {status === 'idle' ? 'Not Running' : status === 'alice' ? '👩 Alice (Receiver)' : '👨 Bob (Sender)'}
        </Text>
      </View>
      
      {/* Buttons */}
      <View style={styles.buttonRow}>
        <TouchableOpacity
          style={[
            styles.button,
            styles.aliceButton,
            (isRunning && status !== 'alice') && styles.buttonDisabled,
          ]}
          onPress={startAlice}
          disabled={isRunning && status !== 'alice'}>
          <Text style={styles.buttonText}>
            {status === 'alice' && isRunning ? '👩 Alice Running' : '👩 Start Alice'}
          </Text>
        </TouchableOpacity>
        
        <TouchableOpacity
          style={[
            styles.button,
            styles.bobButton,
            (isRunning && status !== 'bob') && styles.buttonDisabled,
          ]}
          onPress={startBob}
          disabled={isRunning && status !== 'bob'}>
          <Text style={styles.buttonText}>
            {status === 'bob' && isRunning ? '👨 Bob Running' : '👨 Start Bob'}
          </Text>
        </TouchableOpacity>
      </View>
      
      {/* Stop Button */}
      {isRunning && (
        <TouchableOpacity
          style={[styles.stopButton]}
          onPress={stop}>
          <Text style={styles.stopButtonText}>🛑 Stop</Text>
        </TouchableOpacity>
      )}
      
      {/* Message Log */}
      <View style={styles.messageContainer}>
        <Text style={[styles.messageTitle, {color: isDarkMode ? '#fff' : '#000'}]}>
          Messages:
        </Text>
        <ScrollView
          style={[
            styles.messageLog,
            {backgroundColor: isDarkMode ? '#222' : '#fff'},
          ]}>
          {messages.length === 0 ? (
            <Text style={[styles.noMessages, {color: isDarkMode ? '#888' : '#aaa'}]}>
              No messages yet...
            </Text>
          ) : (
            messages.map(msg => (
              <View
                key={msg.id}
                style={[
                  styles.message,
                  msg.type === 'sent' && styles.messageSent,
                  msg.type === 'received' && styles.messageReceived,
                  msg.type === 'system' && styles.messageSystem,
                ]}>
                <Text
                  style={[
                    styles.messageText,
                    {color: isDarkMode ? '#fff' : '#000'},
                  ]}>
                  {msg.text}
                </Text>
                <Text style={styles.messageTime}>
                  {msg.timestamp.toLocaleTimeString()}
                </Text>
              </View>
            ))
          )}
        </ScrollView>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    padding: 20,
  },
  title: {
    fontSize: 24,
    fontWeight: 'bold',
    marginBottom: 20,
    textAlign: 'center',
  },
  inputContainer: {
    marginBottom: 15,
  },
  label: {
    fontSize: 14,
    marginBottom: 5,
    fontWeight: '500',
  },
  input: {
    borderWidth: 1,
    borderColor: '#ccc',
    borderRadius: 8,
    padding: 12,
    fontSize: 14,
  },
  statusContainer: {
    marginBottom: 15,
    padding: 12,
    borderRadius: 8,
    backgroundColor: 'rgba(0, 122, 255, 0.1)',
  },
  status: {
    fontSize: 16,
    fontWeight: '600',
    textAlign: 'center',
  },
  buttonRow: {
    flexDirection: 'row',
    gap: 10,
    marginBottom: 10,
  },
  button: {
    flex: 1,
    padding: 15,
    borderRadius: 8,
    alignItems: 'center',
  },
  aliceButton: {
    backgroundColor: '#34C759',
  },
  bobButton: {
    backgroundColor: '#007AFF',
  },
  stopButton: {
    backgroundColor: '#FF3B30',
    paddingVertical: 8,
    paddingHorizontal: 20,
    borderRadius: 8,
    alignItems: 'center',
    alignSelf: 'center',
    marginBottom: 10,
  },
  stopButtonText: {
    color: '#fff',
    fontSize: 14,
    fontWeight: '600',
  },
  buttonDisabled: {
    opacity: 0.5,
  },
  buttonText: {
    color: '#fff',
    fontSize: 16,
    fontWeight: '600',
  },
  messageContainer: {
    flex: 1,
    marginTop: 10,
  },
  messageTitle: {
    fontSize: 18,
    fontWeight: '600',
    marginBottom: 10,
  },
  messageLog: {
    flex: 1,
    borderRadius: 8,
    padding: 10,
  },
  noMessages: {
    textAlign: 'center',
    padding: 20,
    fontStyle: 'italic',
  },
  message: {
    padding: 10,
    marginBottom: 8,
    borderRadius: 8,
    borderLeftWidth: 3,
  },
  messageSent: {
    backgroundColor: 'rgba(0, 122, 255, 0.1)',
    borderLeftColor: '#007AFF',
  },
  messageReceived: {
    backgroundColor: 'rgba(52, 199, 89, 0.1)',
    borderLeftColor: '#34C759',
  },
  messageSystem: {
    backgroundColor: 'rgba(142, 142, 147, 0.1)',
    borderLeftColor: '#8E8E93',
  },
  messageText: {
    fontSize: 14,
    marginBottom: 4,
  },
  messageTime: {
    fontSize: 11,
    color: '#8E8E93',
  },
});
