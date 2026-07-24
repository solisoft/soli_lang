package {{JAVA_PACKAGE}};

import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Intent;
import android.net.Uri;
import android.os.Build;
import androidx.core.app.NotificationCompat;
import com.google.firebase.messaging.FirebaseMessagingService;
import com.google.firebase.messaging.RemoteMessage;

public class SoliFirebaseMessagingService extends FirebaseMessagingService {
    private static final String CHANNEL = "{{SCHEME}}_fcm";

    @Override
    public void onNewToken(String token) {
        // MainActivity re-registers on next page load with session cookie.
    }

    @Override
    public void onMessageReceived(RemoteMessage message) {
        String title = message.getNotification() != null
            ? message.getNotification().getTitle()
            : message.getData().get("title");
        String body = message.getNotification() != null
            ? message.getNotification().getBody()
            : message.getData().get("body");
        String url = message.getData().get("url");
        if (url == null || url.isEmpty()) url = MainActivity.START_URL;

        NotificationManager nm = getSystemService(NotificationManager.class);
        if (Build.VERSION.SDK_INT >= 26) {
            nm.createNotificationChannel(new NotificationChannel(
                CHANNEL, "{{APP_NAME}}", NotificationManager.IMPORTANCE_DEFAULT));
        }
        Intent open = new Intent(this, MainActivity.class);
        String target = url.startsWith("http") ? url : MainActivity.START_URL.replaceAll("/$", "") + url;
        open.setData(Uri.parse(target));
        open.addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP);
        PendingIntent pi = PendingIntent.getActivity(
            this, 0, open, PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        NotificationCompat.Builder b = new NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle(title != null ? title : "{{APP_NAME}}")
            .setContentText(body != null ? body : "")
            .setContentIntent(pi)
            .setAutoCancel(true);
        String badge = message.getData().get("badge");
        if (badge != null) {
            try { b.setNumber(Integer.parseInt(badge)); } catch (Exception ignored) {}
        }
        nm.notify((int) System.currentTimeMillis(), b.build());
    }
}
