package app.nearvolt

import android.content.Intent
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContract
import androidx.activity.result.contract.ActivityResultContracts
import io.flutter.embedding.android.FlutterFragmentActivity
import io.flutter.embedding.engine.FlutterEngine
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlin.coroutines.resume

class MainActivity : FlutterFragmentActivity() {
    // Activity result launchers must be registered before the activity starts.
    val pickFiles = ResultLauncher(this, ActivityResultContracts.OpenMultipleDocuments())
    val pickFolder = ResultLauncher(this, ActivityResultContracts.OpenDocumentTree())
    val permissions = ResultLauncher(this, ActivityResultContracts.RequestMultiplePermissions())
    val settings = ResultLauncher(this, ActivityResultContracts.StartActivityForResult())

    private var host: PlatformHost? = null

    override fun onCreate(savedInstanceState: android.os.Bundle?) {
        super.onCreate(savedInstanceState)
        // The app is open, so the "updated, tap to open" notice has done its job.
        UpdatedReceiver.dismiss(this)
    }

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        NativeBridge.register(applicationContext)
        val messenger = flutterEngine.dartExecutor.binaryMessenger
        // Every call the app makes into Android is written to the log (see CallLog).
        host = PlatformHost(this, NearvoltEvents(messenger)).also { NearvoltPlatform.setUp(messenger, CallLog.wrap(it)) }
    }

    override fun onPause() {
        AppLog.write("activity", "paused")
        super.onPause()
    }

    override fun onStop() {
        AppLog.write("activity", "stopped")
        super.onStop()
    }

    override fun onResume() {
        AppLog.write("activity", "resumed")
        super.onResume()
        host?.onResume()
    }

    override fun onNewIntent(intent: Intent) {
        AppLog.write("activity", "new intent ${intent.action}")
        super.onNewIntent(intent)
        host?.onNewIntent(intent)
    }

    override fun onDestroy() {
        AppLog.write("activity", "destroyed (changing configuration: $isChangingConfigurations)")
        host?.close()
        super.onDestroy()
    }
}

/** An activity result launcher you can await. */
class ResultLauncher<I, O>(activity: ComponentActivity, contract: ActivityResultContract<I, O>) {
    private var pending: CancellableContinuation<O>? = null
    private val launcher = activity.registerForActivityResult(contract) { result ->
        pending?.resume(result)
        pending = null
    }

    suspend fun launch(input: I): O = suspendCancellableCoroutine { continuation ->
        // A second request replaces the first, which would otherwise wait for ever.
        pending?.cancel()
        pending = continuation
        launcher.launch(input)
    }
}
