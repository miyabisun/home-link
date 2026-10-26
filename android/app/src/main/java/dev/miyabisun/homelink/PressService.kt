package dev.miyabisun.homelink

import android.app.AlarmManager
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.content.BroadcastReceiver
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import java.util.concurrent.atomic.AtomicInteger

/**
 * Sends every widget press as a short foreground service: Android lets a background app use metered networks
 * with Data Saver or Battery Saver on only from the foreground, and a widget's broadcast receiver is background.
 * A press may also wait longer for its network and answer than the 10 s a receiver may run.
 */
class PressService : Service() {
    companion object {
        private const val CHANNEL = "press"
        private const val TITLE = "title"
        private const val ICON = "icon"
        /** The presses running now; the service stops when none is left. */
        private val running = AtomicInteger()

        /** Each kind of press by the first segment of its key; the press runs off the main thread. */
        private val PRESSES: Map<String, (Context, Intent) -> Unit> = mapOf(
            "door" to ::pressDoor,
            "label" to LabelWidget::press,
            "aircon" to { context, _ -> AirconWidget.press(context) },
        )

        /** Whether a press is under way in this process, which will take its button back itself. */
        val busy: Boolean get() = running.get() > 0

        /**
         * What a widget button sends to run a press: `key`, `<kind>:<button>`, tells the buttons apart, and
         * `title` with `icon` names the press in its notification while it runs.
         */
        fun pending(context: Context, key: String, title: String, icon: Int, extras: Intent.() -> Unit = {}): PendingIntent =
            PendingIntent.getForegroundService(context, 0, Intent(context, PressService::class.java).setAction(key)
                .putExtra(TITLE, title).putExtra(ICON, icon).apply(extras), PendingIntent.FLAG_IMMUTABLE)
    }

    private val main = Handler(Looper.getMainLooper())

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val press = intent?.action?.substringBefore(':')?.let(PRESSES::get) ?: run {
            if (running.get() == 0) stopSelf(startId)
            return START_NOT_STICKY
        }
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(CHANNEL, getString(R.string.press_channel), NotificationManager.IMPORTANCE_LOW))
        // The door's own channel, until 0.1.50.
        manager.deleteNotificationChannel("door")
        val notification = Notification.Builder(this, CHANNEL).setSmallIcon(intent.getIntExtra(ICON, R.drawable.ic_power))
            .setContentTitle(intent.getStringExtra(TITLE)).build()
        try {
            startForeground(1, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE)
        } catch (e: IllegalStateException) {
            // Refused (ForegroundServiceStartNotAllowedException): sent from the background, where Data Saver may
            // block it; the press then shows that as such.
            Log.w(LOG, "${intent.action}: not in the foreground: $e")
        }
        running.incrementAndGet()
        Thread {
            try {
                press(this, intent)
            } finally {
                main.post { if (running.decrementAndGet() == 0) stopSelf() }
            }
        }.start()
        return START_NOT_STICKY
    }

    // A short service may run 3 minutes; a press takes at most about a minute.
    override fun onTimeout(startId: Int, fgsType: Int) = stopSelf()
}

/** The tag of the app's log lines. */
internal const val LOG = "home-link"

/**
 * One button's presses, each shown through to its end: [SENDING], the result for [RESULT_MS], then the button
 * again, unless the button was pressed again meanwhile.
 */
internal class PressFlow {
    companion object {
        /** How long a button shows a press's result before it reads as a button again. */
        const val RESULT_MS = 5_000L
    }

    private val presses = AtomicInteger()

    /**
     * Runs one press: `sending` draws it under way; `send` sends it and draws the result, true when that result is
     * to be held; `reread` reads the state again; `restore` draws the button again.
     */
    fun run(context: Context, sending: () -> Unit, send: () -> Boolean, reread: () -> Unit, restore: () -> Unit) {
        val press = presses.incrementAndGet()
        sending()
        val hold = send()
        val shown = SystemClock.elapsedRealtime()
        // Should the process die while the result shows, the alarm takes the button back.
        if (hold) Widgets.restoreAt(context, shown + RESULT_MS)
        reread()
        if (hold) Thread.sleep((shown + RESULT_MS - SystemClock.elapsedRealtime()).coerceAtLeast(0))
        if (presses.get() == press) restore()
    }
}

/** What every kind of home-link widget shares: redrawing them all, reading their states again, and when. */
internal object Widgets {
    private const val RESTORE = "dev.miyabisun.homelink.RESTORE"
    private const val REFRESH_JOB = 1

    /** Draws every placed widget as last known. */
    fun showAll(context: Context) {
        LabelWidget.showAll(context)
        AirconWidget.showAll(context)
        showDoors(context)
    }

    /**
     * Has Android read the states again as soon as a network this app may use is up, as when the phone joins the
     * home Wi-Fi after mobile data with Data Saver on, rather than at the next half-hourly update.
     */
    fun refreshWhenOnline(context: Context) {
        context.getSystemService(JobScheduler::class.java).schedule(
            JobInfo.Builder(REFRESH_JOB, ComponentName(context, WidgetRefresh::class.java))
                .setRequiredNetworkType(JobInfo.NETWORK_TYPE_ANY)
                .setBackoffCriteria(30_000, JobInfo.BACKOFF_POLICY_EXPONENTIAL).setPersisted(true).build())
    }

    /** Takes every button back at `elapsed` ms of [SystemClock.elapsedRealtime], even if this process is gone then. */
    fun restoreAt(context: Context, elapsed: Long) {
        context.getSystemService(AlarmManager::class.java).set(AlarmManager.ELAPSED_REALTIME, elapsed,
            PendingIntent.getBroadcast(context, 0, Intent(context, WidgetEvents::class.java).setAction(RESTORE), PendingIntent.FLAG_IMMUTABLE))
    }

    // A press still running in this process takes its own button back.
    fun onEvent(context: Context, intent: Intent) {
        if (intent.action == RESTORE && !PressService.busy) showAll(context)
    }
}

/** Receives the alarm of [Widgets.restoreAt]. */
class WidgetEvents : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) = Widgets.onEvent(context, intent)
}

/** Reads the widgets' states once Android sees a network this app may use; again later while that read fails. */
class WidgetRefresh : JobService() {
    override fun onStartJob(params: JobParameters): Boolean {
        Thread {
            val read = LabelWidget.read(this) and AirconWidget.read(this)
            if (!PressService.busy) Widgets.showAll(this)
            Log.i(LOG, "widgets' states read again on a network: ${if (read) "done" else "failed, later again"}")
            jobFinished(params, !read)
        }.start()
        return true
    }

    override fun onStopJob(params: JobParameters) = true
}

/** Runs `work` off the main thread while the receiver stays alive for it. */
internal fun BroadcastReceiver.async(work: () -> Unit) {
    val pending = goAsync()
    Thread {
        try {
            work()
        } finally {
            pending.finish()
        }
    }.start()
}
