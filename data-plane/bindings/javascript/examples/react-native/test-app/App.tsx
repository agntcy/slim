import React, {useState} from 'react';
import {
  SafeAreaView,
  ScrollView,
  StatusBar,
  StyleSheet,
  Text,
  TouchableOpacity,
  View,
  useColorScheme,
} from 'react-native';

// Import SLIM bindings
// Note: These will be available once the app is built with the native module
let slimBindings: any = null;
let bindingsError: string = '';

async function initializeSlimBindings() {
  try {
    console.log('[DEBUG] Attempting to require @agntcy/slim-bindings...');
    const bindings = require('@agntcy/slim-bindings');
    console.log('[DEBUG] Module required successfully');
    console.log('[DEBUG] Available exports:', Object.keys(bindings).slice(0, 20).join(', '));
    
    // Check if JSI bindings are available
    console.log('[DEBUG] Checking for JSI bindings availability...');
    if (bindings.isJSIBindingsAvailable && bindings.isJSIBindingsAvailable()) {
      console.log('[DEBUG] JSI bindings are immediately available');
      slimBindings = bindings;
    } else if (bindings.waitForJSIBindings) {
      console.log('[DEBUG] Waiting for JSI bindings to be installed...');
      await bindings.waitForJSIBindings(5000);
      console.log('[DEBUG] JSI bindings are now available');
      slimBindings = bindings;
    } else {
      bindingsError = 'JSI bindings helper functions not found';
      console.error('[ERROR] ' + bindingsError);
      // Try using it anyway
      slimBindings = bindings;
    }
  } catch (e: any) {
    bindingsError = e.message || String(e);
    console.error('[ERROR] SLIM bindings initialization failed:', e);
    console.error('[ERROR] Error stack:', e.stack);
  }
  
  console.log('[DEBUG] Final state - slimBindings:', slimBindings ? 'LOADED' : 'NULL', 'error:', bindingsError);
}

// Initialize bindings on module load
initializeSlimBindings().catch(e => {
  console.error('[ERROR] Failed to initialize SLIM bindings:', e);
});

interface TestResult {
  name: string;
  status: 'PASS' | 'FAIL' | 'SKIP';
  details?: string;
  error?: string;
}

function App(): React.JSX.Element {
  const isDarkMode = useColorScheme() === 'dark';
  const [status, setStatus] = useState<string>('Not Started');
  const [testResults, setTestResults] = useState<TestResult[]>([]);
  const [isRunning, setIsRunning] = useState(false);

  const backgroundStyle = {
    backgroundColor: isDarkMode ? '#000' : '#fff',
    flex: 1,
  };

  const runTests = async () => {
    console.log('[DEBUG] runTests called, slimBindings:', slimBindings ? 'LOADED' : 'NULL');
    setIsRunning(true);
    setStatus('Running');
    const results: TestResult[] = [];

    try {
      if (!slimBindings) {
        console.error('[ERROR] No slim bindings, error was:', bindingsError);
        results.push({
          name: 'bindings-load',
          status: 'FAIL',
          error: bindingsError || 'SLIM bindings module not loaded',
        });
        setStatus('Failed');
        setTestResults(results);
        setIsRunning(false);
        return;
      }

      console.log('[DEBUG] Testing initializeWithDefaults...');
      // Test 1: Initialize runtime with defaults
      try {
        slimBindings.initializeWithDefaults();
        results.push({name: 'init-runtime', status: 'PASS'});
      } catch (error: any) {
        results.push({
          name: 'init-runtime',
          status: 'FAIL',
          error: error.message,
        });
      }

      // Test 2: Get version
      try {
        const version = slimBindings.getVersion();
        results.push({
          name: 'get-version',
          status: version ? 'PASS' : 'FAIL',
          details: version || 'No version returned',
        });
      } catch (error: any) {
        results.push({
          name: 'get-version',
          status: 'FAIL',
          error: error.message,
        });
      }

      // Test 3: Create Name
      try {
        const name = slimBindings.Name.newWithId('org', 'test', 'v1', 123n);
        const nameStr = name.toString();
        const isValid = nameStr.startsWith('org/test/v1');
        results.push({
          name: 'create-name',
          status: isValid ? 'PASS' : 'FAIL',
          details: `Name: ${nameStr}`,
        });
      } catch (error: any) {
        results.push({
          name: 'create-name',
          status: 'FAIL',
          error: error.message,
        });
      }

      // Test 4: Create Service and App with SharedSecret
      try {
        const service = slimBindings.getGlobalService();
        const appName = slimBindings.Name.newWithId('org', 'app', 'v1', 456n);
        const app = service.createAppWithSecret(
          appName,
          'test-secret-must-be-at-least-32-bytes!',
        );
        results.push({
          name: 'create-app',
          status: app ? 'PASS' : 'FAIL',
          details: app ? `App created` : 'No app returned',
        });
      } catch (error: any) {
        results.push({
          name: 'create-app',
          status: 'FAIL',
          error: error.message,
        });
      }

      // Determine overall status
      const hasFailures = results.some(r => r.status === 'FAIL');
      setStatus(hasFailures ? 'Failed' : 'Complete');
    } catch (error: any) {
      results.push({
        name: 'error',
        status: 'FAIL',
        error: error.message,
      });
      setStatus('Failed');
    }

    setTestResults(results);
    setIsRunning(false);
  };

  const getStatusColor = () => {
    if (status === 'Not Started') return '#666';
    if (status === 'Running') return '#007AFF';
    if (status === 'Complete') return '#34C759';
    return '#FF3B30';
  };

  const getResultColor = (result: TestResult) => {
    if (result.status === 'PASS') return '#34C759';
    if (result.status === 'FAIL') return '#FF3B30';
    return '#FFB000';
  };

  return (
    <SafeAreaView style={backgroundStyle}>
      <StatusBar
        barStyle={isDarkMode ? 'light-content' : 'dark-content'}
        backgroundColor={backgroundStyle.backgroundColor}
      />
      <ScrollView
        contentInsetAdjustmentBehavior="automatic"
        style={backgroundStyle}>
        <View
          style={[
            styles.container,
            {backgroundColor: isDarkMode ? '#111' : '#f5f5f5'},
          ]}>
          <Text style={[styles.title, {color: isDarkMode ? '#fff' : '#000'}]}>
            SLIM Bindings Test
          </Text>

          {/* Show binding error if there is one */}
          {bindingsError ? (
            <View style={styles.errorContainer}>
              <Text testID="binding-error" style={styles.errorText}>
                ⚠️ Binding Error: {bindingsError}
              </Text>
            </View>
          ) : null}

          <View style={styles.statusContainer}>
            <Text
              testID="status"
              style={[styles.status, {color: getStatusColor()}]}>
              Status: {status}
            </Text>
          </View>

          <TouchableOpacity
            testID="run-tests"
            style={[
              styles.button,
              isRunning && styles.buttonDisabled,
              {backgroundColor: isDarkMode ? '#007AFF' : '#007AFF'},
            ]}
            onPress={runTests}
            disabled={isRunning}>
            <Text style={styles.buttonText}>
              {isRunning ? 'Running Tests...' : 'Run Tests'}
            </Text>
          </TouchableOpacity>

          <View style={styles.resultsContainer}>
            <Text
              style={[
                styles.resultsTitle,
                {color: isDarkMode ? '#fff' : '#000'},
              ]}>
              Test Results:
            </Text>
            {testResults.length === 0 ? (
              <Text
                style={[
                  styles.noResults,
                  {color: isDarkMode ? '#888' : '#666'},
                ]}>
                No tests run yet
              </Text>
            ) : (
              testResults.map((result, i) => (
                <View key={i} style={styles.resultItem}>
                  <Text
                    testID={`result-${result.name}`}
                    style={[
                      styles.resultText,
                      {
                        color: isDarkMode ? '#fff' : '#000',
                      },
                    ]}>
                    <Text style={{color: getResultColor(result)}}>
                      {result.status}
                    </Text>
                    {' - '}
                    {result.name}
                  </Text>
                  {result.details && (
                    <Text
                      style={[
                        styles.resultDetails,
                        {color: isDarkMode ? '#aaa' : '#666'},
                      ]}>
                      {result.details}
                    </Text>
                  )}
                  {result.error && (
                    <Text style={[styles.resultDetails, {color: '#FF3B30'}]}>
                      Error: {result.error}
                    </Text>
                  )}
                </View>
              ))
            )}
          </View>
        </View>
      </ScrollView>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  container: {
    padding: 20,
    minHeight: '100%',
  },
  title: {
    fontSize: 24,
    fontWeight: 'bold',
    marginBottom: 20,
    textAlign: 'center',
  },
  errorContainer: {
    backgroundColor: '#ffebee',
    padding: 15,
    borderRadius: 8,
    marginBottom: 20,
    borderLeftWidth: 4,
    borderLeftColor: '#f44336',
  },
  errorText: {
    color: '#c62828',
    fontSize: 14,
    fontFamily: 'monospace',
  },
  statusContainer: {
    marginBottom: 20,
    padding: 15,
    borderRadius: 8,
    backgroundColor: '#f9f9f9',
  },
  status: {
    fontSize: 18,
    fontWeight: '600',
    textAlign: 'center',
  },
  button: {
    padding: 15,
    borderRadius: 8,
    marginBottom: 20,
  },
  buttonDisabled: {
    opacity: 0.6,
  },
  buttonText: {
    color: '#fff',
    fontSize: 16,
    fontWeight: '600',
    textAlign: 'center',
  },
  resultsContainer: {
    marginTop: 10,
  },
  resultsTitle: {
    fontSize: 18,
    fontWeight: '600',
    marginBottom: 10,
  },
  noResults: {
    fontSize: 14,
    fontStyle: 'italic',
    textAlign: 'center',
    paddingVertical: 20,
  },
  resultItem: {
    marginBottom: 15,
    padding: 10,
    borderLeftWidth: 3,
    borderLeftColor: '#007AFF',
    backgroundColor: 'rgba(0, 122, 255, 0.05)',
  },
  resultText: {
    fontSize: 16,
    fontWeight: '500',
    marginBottom: 4,
  },
  resultDetails: {
    fontSize: 14,
    marginTop: 4,
    paddingLeft: 10,
  },
});

export default App;
