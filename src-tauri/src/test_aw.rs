// Test program to verify ActivityWatch client works
use chrono::Utc;

#[tokio::main]
async fn main() {
    println!("Testing ActivityWatch client...");
    
    // Import the module
    let aw_client = crate::modules::activity_watch::ActivityWatchClient::new(
        "localhost".to_string(), 
        5600
    );
    
    // Test connection
    let conn_status = aw_client.test_connection().await;
    println!("Connection status: {:?}", conn_status);
    
    if conn_status.connected {
        println!("Connected! Testing get_multi_timeframe_data_active...");
        
        match aw_client.get_multi_timeframe_data_active().await {
            Ok(data) => {
                println!("Success! Got {} timeframes", data.len());
                for (name, tf) in data.iter() {
                    println!("  {}: {} window events, {} afk events", 
                        name, 
                        tf.window_events.len(),
                        tf.afk_events.len()
                    );
                }
            },
            Err(e) => {
                println!("Error: {}", e);
            }
        }
    }
}