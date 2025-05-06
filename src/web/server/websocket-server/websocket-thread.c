#include "daemon/common.h"
#include "websocket-internal.h"
#include "poll.h"

// Global array of WebSocket threads
WEBSOCKET_THREAD websocket_threads[WEBSOCKET_MAX_THREADS];

// Initialize WebSocket thread system
void websocket_threads_init(void) {
    for(size_t i = 0; i < WEBSOCKET_MAX_THREADS; i++) {
        websocket_threads[i].id = i;
        websocket_threads[i].thread = NULL;
        websocket_threads[i].running = false;
        spinlock_init(&websocket_threads[i].spinlock);
        websocket_threads[i].clients_current = 0;
        spinlock_init(&websocket_threads[i].clients_spinlock);
        websocket_threads[i].clients_first = NULL;
        websocket_threads[i].clients_last = NULL;
        websocket_threads[i].ndpl = NULL;
        websocket_threads[i].cmd.pipe[PIPE_READ] = -1;
        websocket_threads[i].cmd.pipe[PIPE_WRITE] = -1;
        websocket_threads[i].cmd.slot = 0;
    }
}

// Find the thread with the minimum client load and atomically increment its count
NEVERNULL
static WEBSOCKET_THREAD *websocket_thread_get_min_load(void) {
    // Static spinlock to protect the critical section of thread selection
    static SPINLOCK assign_spinlock = SPINLOCK_INITIALIZER;
    size_t slot = 0;
    
    // Critical section: find thread with minimum load and increment its count atomically
    spinlock_lock(&assign_spinlock);
    
    // Find the minimum load thread
    size_t min_clients = websocket_threads[0].clients_current;
    
    for(size_t i = 1; i < WEBSOCKET_MAX_THREADS; i++) {
        // Check if this thread has fewer clients
        if(websocket_threads[i].clients_current < min_clients) {
            min_clients = websocket_threads[i].clients_current;
            slot = i;
        }
    }
    
    // Preemptively increment the client count to prevent race conditions
    // This ensures concurrent client assignments will be properly distributed
    websocket_threads[slot].clients_current++;
    
    spinlock_unlock(&assign_spinlock);
    
    return &websocket_threads[slot];
}

// Initialize a thread's poll
static bool websocket_thread_init_poll(WEBSOCKET_THREAD *wth) {
    if(!wth) return false;

    // Create poll instance
    if(!wth->ndpl) {
        wth->ndpl = nd_poll_create();
        if (!wth->ndpl) {
            netdata_log_error("WEBSOCKET: Failed to create poll for thread %zu", wth->id);
            goto cleanup;
        }
    }

    // Create command pipe
    if(wth->cmd.pipe[PIPE_READ] == -1 || wth->cmd.pipe[PIPE_WRITE] == -1) {
        if (pipe(wth->cmd.pipe) == -1) {
            netdata_log_error("WEBSOCKET: Failed to create command pipe for thread %zu: %s", wth->id, strerror(errno));
            goto cleanup;
        }

        // Set pipe to non-blocking
        if(fcntl(wth->cmd.pipe[PIPE_READ], F_SETFL, O_NONBLOCK) == -1) {
            netdata_log_error("WEBSOCKET: Failed to set command pipe to non-blocking for thread %zu: %s", wth->id, strerror(errno));
            goto cleanup;
        }

        // Add command pipe to poll
        bool added = nd_poll_add(wth->ndpl, wth->cmd.pipe[PIPE_READ], ND_POLL_READ, &wth->cmd);
        if(!added) {
            netdata_log_error("WEBSOCKET: Failed to add command pipe to poll for thread %zu", wth->id);
            goto cleanup;
        }
    }

    return true;

cleanup:
    if(wth->cmd.pipe[PIPE_READ] != -1) {
        close(wth->cmd.pipe[PIPE_READ]);
        wth->cmd.pipe[PIPE_READ] = -1;
    }
    if(wth->cmd.pipe[PIPE_WRITE] != -1) {
        close(wth->cmd.pipe[PIPE_WRITE]);
        wth->cmd.pipe[PIPE_WRITE] = -1;
    }
    if(wth->ndpl) {
        nd_poll_destroy(wth->ndpl);
        wth->ndpl = NULL;
    }
    return false;
}

// Thread main function
void *websocket_thread(void *ptr) {
    WEBSOCKET_THREAD *wth = (WEBSOCKET_THREAD *)ptr;
    
    if(!wth) {
        netdata_log_error("WEBSOCKET: Thread started with NULL argument");
        return NULL;
    }

    netdata_log_info("WEBSOCKET: Thread %zu started", wth->id);

    time_t last_cleanup = now_monotonic_sec();
    
    // Main thread loop
    while(service_running(SERVICE_STREAMING) && !nd_thread_signaled_to_cancel()) {

        // Poll for events
        nd_poll_result_t ev;
        int rc = nd_poll_wait(wth->ndpl, 100000, &ev); // 100ms timeout
        if(rc < 0) {
            if(errno == EAGAIN || errno == EINTR)
                continue;
                
            netdata_log_error("WEBSOCKET: Poll error in thread %zu: %s", wth->id, strerror(errno));
            break;
        }

        // Process poll events
        if(rc > 0) {
            // Handle command pipe
            if(ev.data == &wth->cmd) {
                if(ev.events & ND_POLL_READ) {
                    // Read and process commands
                    websocket_thread_process_commands(wth);
                }
                continue;
            }

            // Handle client events
            WEBSOCKET_SERVER_CLIENT *wsc = (WEBSOCKET_SERVER_CLIENT *)ev.data;
            if(!wsc) {
                netdata_log_error("WEBSOCKET: Poll event with NULL client data in thread %zu", wth->id);
                continue;
            }

            // Check for errors
            if(ev.events & (ND_POLL_ERROR | ND_POLL_HUP)) {
                netdata_log_error("WEBSOCKET: Client %zu socket error or hangup", wsc->id);
                // TODO: Handle client disconnection
                websocket_thread_send_command(wth, WEBSOCKET_THREAD_CMD_REMOVE_CLIENT, &wsc->id, sizeof(wsc->id));
                continue;
            }

            // Process read events
            if(ev.events & ND_POLL_READ) {
                // Handle incoming data
                int result = websocket_receive_data(wsc);
                if(result < 0) {
                    // Error or connection closed
                    netdata_log_error("WEBSOCKET: Failed to receive data from client %zu", wsc->id);
                    websocket_thread_send_command(wth, WEBSOCKET_THREAD_CMD_REMOVE_CLIENT, &wsc->id, sizeof(wsc->id));
                    continue;
                }
            }

            // Process write events
            if(ev.events & ND_POLL_WRITE) {
                // Send pending data
                int result = websocket_write_data(wsc);
                if(result < 0) {
                    // Error writing data
                    netdata_log_error("WEBSOCKET: Failed to write data to client %zu", wsc->id);
                    websocket_thread_send_command(wth, WEBSOCKET_THREAD_CMD_REMOVE_CLIENT, &wsc->id, sizeof(wsc->id));
                    continue;
                }
            }
        }

        // Periodic cleanup and health checks (every 30 seconds)
        time_t now = now_monotonic_sec();
        if(now - last_cleanup > 30) {
            // Iterate through all clients in this thread
            spinlock_lock(&wth->clients_spinlock);
            
            WEBSOCKET_SERVER_CLIENT *wsc = wth->clients_first;
            while(wsc) {
                WEBSOCKET_SERVER_CLIENT *next = wsc->next; // Save next in case we remove this client
                
                if(wsc->state == WS_STATE_OPEN) {
                    // Check if client is idle (no activity for over 120 seconds)
                    if(now - wsc->last_activity_t > 120) {
                        // Client is idle - send a ping to check if it's still alive
                        websocket_ping_client(wsc);
                        
                        // If no activity for over 300 seconds (5 minutes), consider it dead
                        if(now - wsc->last_activity_t > 300) {
                            netdata_log_error("WEBSOCKET: Client %zu timed out (no activity for over 5 minutes)", wsc->id);
                            websocket_close_client(wsc, 1001, "Timeout - no activity");
                            websocket_thread_send_command(wth, WEBSOCKET_THREAD_CMD_REMOVE_CLIENT, &wsc->id, sizeof(wsc->id));
                        }
                    }
                    // For normal clients, send periodic pings (every 60 seconds)
                    else if(now - wsc->last_activity_t > 60) {
                        websocket_ping_client(wsc);
                    }
                }
                else if(wsc->state == WS_STATE_CLOSING) {
                    // If a client is in CLOSING state for more than 5 seconds, force close it
                    if(now - wsc->last_activity_t > 5) {
                        netdata_log_error("WEBSOCKET: Forcing close of client %zu (stuck in CLOSING state)", wsc->id);
                        websocket_thread_send_command(wth, WEBSOCKET_THREAD_CMD_REMOVE_CLIENT, &wsc->id, sizeof(wsc->id));
                    }
                }
                
                wsc = next;
            }
            
            spinlock_unlock(&wth->clients_spinlock);
            
            last_cleanup = now;
        }
    }

    netdata_log_info("WEBSOCKET: Thread %zu exiting", wth->id);

    // Clean up any remaining clients
    spinlock_lock(&wth->clients_spinlock);
    
    // Close all clients in this thread
    WEBSOCKET_SERVER_CLIENT *wsc = wth->clients_first;
    while(wsc) {
        WEBSOCKET_SERVER_CLIENT *next = wsc->next;
        
        netdata_log_info("WEBSOCKET: Closing client %zu during thread shutdown", wsc->id);
        
        // First remove from the poll to prevent further events
        if (wsc->sock.fd >= 0) {
            bool removed = nd_poll_del(wth->ndpl, wsc->sock.fd);
            if (!removed) {
                netdata_log_error("WEBSOCKET: Failed to remove client %zu from poll during shutdown", wsc->id);
            }
        }
        
        // If client is still connected and not in CLOSED state, send close frame
        if(wsc->sock.fd >= 0 && wsc->state != WS_STATE_CLOSED) {
            // Try to send close frame (ignoring errors)
            websocket_close_client(wsc, 1001, "Server shutting down");
            
            // Flush any pending data - best effort, ignore errors
            if(buffer_strlen(wsc->out_buffer) > 0) {
                websocket_write_data(wsc);
            }
        }
        
        // Close socket if still open
        nd_sock_close(&wsc->sock);
        
        // Free buffers
        if(wsc->in_buffer) {
            buffer_free(wsc->in_buffer);
            wsc->in_buffer = NULL;
        }
        
        if(wsc->out_buffer) {
            buffer_free(wsc->out_buffer);
            wsc->out_buffer = NULL;
        }
        
        // Unregister from global client registry
        websocket_client_unregister(wsc);
        
        // Finally free the client structure
        freez(wsc);
        
        wsc = next;
    }
    
    // Reset thread's client list
    wth->clients_first = NULL;
    wth->clients_last = NULL;
    wth->clients_current = 0;
    
    spinlock_unlock(&wth->clients_spinlock);
    
    // Cleanup poll resources
    if(wth->ndpl) {
        nd_poll_destroy(wth->ndpl);
        wth->ndpl = NULL;
    }

    // Cleanup command pipe
    if(wth->cmd.pipe[PIPE_READ] != -1) {
        close(wth->cmd.pipe[PIPE_READ]);
        wth->cmd.pipe[PIPE_READ] = -1;
    }

    if(wth->cmd.pipe[PIPE_WRITE] != -1) {
        close(wth->cmd.pipe[PIPE_WRITE]);
        wth->cmd.pipe[PIPE_WRITE] = -1;
    }
    
    // Mark thread as not running
    spinlock_lock(&wth->spinlock);
    wth->running = false;
    spinlock_unlock(&wth->spinlock);

    return NULL;
}

// Assign a client to a thread
WEBSOCKET_THREAD *websocket_thread_assign_client(WEBSOCKET_SERVER_CLIENT *wsc) {
    if(!wsc)
        return NULL;

    // Get the thread with the minimum load 
    // Note: client count is already atomically incremented inside this function
    WEBSOCKET_THREAD *wth = websocket_thread_get_min_load();

    // Lock the thread for initialization
    spinlock_lock(&wth->spinlock);

    // Start the thread if not running
    if(!wth->thread) {
        // Initialize poll
        if(!websocket_thread_init_poll(wth)) {
            spinlock_unlock(&wth->spinlock);
            netdata_log_error("WEBSOCKET: Failed to initialize poll for thread %zu", wth->id);
            goto undo;
        }

        char thread_name[32];
        snprintf(thread_name, sizeof(thread_name), "WEBSOCK[%zu]", wth->id);
        wth->thread = nd_thread_create(thread_name, NETDATA_THREAD_OPTION_DEFAULT, websocket_thread, wth);
        wth->running = true;
    }

    // Release the thread lock
    spinlock_unlock(&wth->spinlock);

    // Send command to add client
    if(!websocket_thread_send_command(wth, WEBSOCKET_THREAD_CMD_ADD_CLIENT, &wsc->id, sizeof(wsc->id))) {
        netdata_log_error("WEBSOCKET: Failed to send add client command to thread %zu", wth->id);
        goto undo;
    }

    // Link thread to client
    wsc->thread = wth;

    return wth;

undo:
    // Roll back the client count increment since assignment failed
    if(wth) {
        spinlock_lock(&wth->clients_spinlock);
        if (wth->clients_current > 0)
            wth->clients_current--;
        spinlock_unlock(&wth->clients_spinlock);
    }

    return NULL;
}

// Add a client to a thread's poll
bool websocket_thread_add_client_to_poll(WEBSOCKET_THREAD *wth, WEBSOCKET_SERVER_CLIENT *wsc) {
    if(!wth || !wsc || wsc->sock.fd < 0)
        return false;

    // Lock the thread poll
    spinlock_lock(&wth->spinlock);

    // Add client to the poll - use the socket fd directly
    bool added = nd_poll_add(wth->ndpl, wsc->sock.fd, ND_POLL_READ, wsc);
    if(!added) {
        netdata_log_error("WEBSOCKET: Failed to add client %zu to poll", wsc->id);
        spinlock_unlock(&wth->spinlock);
        return false;
    }

    // Release the thread poll lock
    spinlock_unlock(&wth->spinlock);

    // Add client to the thread's client list
    websocket_thread_enqueue_client(wth, wsc);

    return true;
}

// Remove a client from a thread's poll
void websocket_thread_remove_client_from_poll(WEBSOCKET_THREAD *wth, WEBSOCKET_SERVER_CLIENT *wsc) {
    if(!wth || !wsc || wsc->sock.fd < 0)
        return;

    // Lock the thread poll
    spinlock_lock(&wth->spinlock);

    // Remove client from the poll - use socket fd directly
    bool removed = nd_poll_del(wth->ndpl, wsc->sock.fd);
    if(!removed) {
        netdata_log_error("WEBSOCKET: Failed to remove client %zu from poll", wsc->id);
    }

    // Release the thread poll lock
    spinlock_unlock(&wth->spinlock);

    // Remove client from the thread's client list
    websocket_thread_dequeue_client(wth, wsc);
}

// Thread client enqueue function
void websocket_thread_enqueue_client(WEBSOCKET_THREAD *wth, WEBSOCKET_SERVER_CLIENT *wsc) {
    if(!wth || !wsc)
        return;

    // Lock the thread clients
    spinlock_lock(&wth->clients_spinlock);

    // Add client to the thread's client list
    if(!wth->clients_first) {
        wth->clients_first = wsc;
        wth->clients_last = wsc;
        wsc->prev = NULL;
        wsc->next = NULL;
    } else {
        wsc->prev = wth->clients_last;
        wsc->next = NULL;
        wth->clients_last->next = wsc;
        wth->clients_last = wsc;
    }

    // Note: clients_current is already incremented in websocket_thread_assign_client
    // to ensure proper load distribution in concurrent scenarios

    // Release the thread clients lock
    spinlock_unlock(&wth->clients_spinlock);
}

// Thread client dequeue function
void websocket_thread_dequeue_client(WEBSOCKET_THREAD *wth, WEBSOCKET_SERVER_CLIENT *wsc) {
    if(!wth || !wsc)
        return;

    // Lock the thread clients
    spinlock_lock(&wth->clients_spinlock);

    // Remove client from the thread's client list
    if(wsc->prev)
        wsc->prev->next = wsc->next;
    else
        wth->clients_first = wsc->next;

    if(wsc->next)
        wsc->next->prev = wsc->prev;
    else
        wth->clients_last = wsc->prev;

    wsc->prev = NULL;
    wsc->next = NULL;

    if(wth->clients_current > 0)
        wth->clients_current--;

    // Release the thread clients lock
    spinlock_unlock(&wth->clients_spinlock);
}

// Update a client's poll event flags
bool websocket_thread_update_client_poll_flags(WEBSOCKET_THREAD *wth, WEBSOCKET_SERVER_CLIENT *wsc, nd_poll_event_t events) {
    if(!wth || !wsc || wsc->sock.fd < 0)
        return false;

    // Lock the thread poll
    spinlock_lock(&wth->spinlock);

    // Update poll events
    bool updated = nd_poll_upd(wth->ndpl, wsc->sock.fd, events);
    if(!updated) {
        netdata_log_error("WEBSOCKET: Failed to update poll events for client %zu", wsc->id);
    }

    // Release the thread poll lock
    spinlock_unlock(&wth->spinlock);

    return updated;
}

// Send command to a thread
bool websocket_thread_send_command(WEBSOCKET_THREAD *wth, uint8_t cmd, void *data, size_t data_len) {
    if(!wth || wth->cmd.pipe[PIPE_WRITE] == -1) {
        netdata_log_error("WEBSOCKET: Failed to send command to thread %zu: pipe is not initialized", wth->id);
        return false;
    }

    // Prepare command
    struct {
        uint8_t cmd;
        size_t len;
        char data[sizeof(size_t)]; // Variable size, just a placeholder
    } __attribute__((packed)) message;

    message.cmd = cmd;
    message.len = data_len;

    // Lock command pipe for writing
    spinlock_lock(&wth->spinlock);

    // Write command header
    ssize_t bytes = write(wth->cmd.pipe[PIPE_WRITE], &message, sizeof(message.cmd) + sizeof(message.len));
    if(bytes != sizeof(message.cmd) + sizeof(message.len)) {
        netdata_log_error("WEBSOCKET: Failed to write command header to thread %zu: %s", wth->id, strerror(errno));
        spinlock_unlock(&wth->spinlock);
        return false;
    }

    // Write command data
    if(data && data_len > 0) {
        bytes = write(wth->cmd.pipe[PIPE_WRITE], data, data_len);
        if(bytes != (ssize_t)data_len) {
            netdata_log_error("WEBSOCKET: Failed to write command data to thread %zu: %s", wth->id, strerror(errno));
            spinlock_unlock(&wth->spinlock);
            return false;
        }
    }

    // Release command pipe
    spinlock_unlock(&wth->spinlock);

    return true;
}

// Process a thread's command pipe
void websocket_thread_process_commands(WEBSOCKET_THREAD *wth) {
    if(!wth || wth->cmd.pipe[PIPE_READ] == -1)
        return;

    struct {
        uint8_t cmd;
        size_t len;
    } __attribute__((packed)) header;

    char buffer[1024]; // Buffer for command data

    // Read all available commands
    for(;;) {
        // Read command header
        ssize_t bytes = read(wth->cmd.pipe[PIPE_READ], &header, sizeof(header));
        if(bytes <= 0) {
            if(bytes < 0 && errno != EAGAIN && errno != EWOULDBLOCK) {
                netdata_log_error("WEBSOCKET: Failed to read command header from pipe: %s", strerror(errno));
            }
            break;
        }

        if(bytes != sizeof(header)) {
            netdata_log_error("WEBSOCKET: Read partial command header (%zd/%zu bytes)", bytes, sizeof(header));
            break;
        }

        // Read command data
        if(header.len > 0) {
            if(header.len > sizeof(buffer)) {
                netdata_log_error("WEBSOCKET: Command data too large (%zu bytes)", header.len);
                // Skip the data
                size_t to_skip = header.len;
                while(to_skip > 0) {
                    size_t chunk = to_skip > sizeof(buffer) ? sizeof(buffer) : to_skip;
                    bytes = read(wth->cmd.pipe[PIPE_READ], buffer, chunk);
                    if(bytes <= 0)
                        break;
                    to_skip -= bytes;
                }
                continue;
            }

            bytes = read(wth->cmd.pipe[PIPE_READ], buffer, header.len);
            if(bytes != (ssize_t)header.len) {
                netdata_log_error("WEBSOCKET: Read partial command data (%zd/%zu bytes)", bytes, header.len);
                break;
            }
        }

        // Process command
        switch(header.cmd) {
            case WEBSOCKET_THREAD_CMD_EXIT:
                netdata_log_info("WEBSOCKET: Thread %zu received exit command", wth->id);
                return;

            case WEBSOCKET_THREAD_CMD_ADD_CLIENT: {
                if(header.len != sizeof(size_t)) {
                    netdata_log_error("WEBSOCKET: Invalid add client command size (%zu != %zu)", header.len, sizeof(size_t));
                    continue;
                }
                size_t client_id = *(size_t *)buffer;
                WEBSOCKET_SERVER_CLIENT *wsc = websocket_client_find_by_id(client_id);
                if(!wsc) {
                    netdata_log_error("WEBSOCKET: Client %zu not found for add command", client_id);
                    continue;
                }
                websocket_thread_add_client_to_poll(wth, wsc);
                break;
            }

            case WEBSOCKET_THREAD_CMD_REMOVE_CLIENT: {
                if(header.len != sizeof(size_t)) {
                    netdata_log_error("WEBSOCKET: Invalid remove client command size (%zu != %zu)", header.len, sizeof(size_t));
                    continue;
                }
                size_t client_id = *(size_t *)buffer;
                WEBSOCKET_SERVER_CLIENT *wsc = websocket_client_find_by_id(client_id);
                if(!wsc) {
                    netdata_log_error("WEBSOCKET: Client %zu not found for remove command", client_id);
                    continue;
                }
                
                // First remove from the poll to prevent further events
                websocket_thread_remove_client_from_poll(wth, wsc);
                
                // If client is still connected and not in CLOSED state, send close frame
                if(wsc->sock.fd >= 0 && wsc->state != WS_STATE_CLOSED) {
                    // Try to send close frame (ignoring errors)
                    websocket_close_client(wsc, 1000, "Connection closed by server");
                    
                    // Flush any pending data - best effort, ignore errors
                    if(buffer_strlen(wsc->out_buffer) > 0) {
                        websocket_write_data(wsc);
                    }
                }
                
                // Close socket if still open
                nd_sock_close(&wsc->sock);
                
                // Free buffers
                if(wsc->in_buffer) {
                    buffer_free(wsc->in_buffer);
                    wsc->in_buffer = NULL;
                }
                
                if(wsc->out_buffer) {
                    buffer_free(wsc->out_buffer);
                    wsc->out_buffer = NULL;
                }
                
                // Unregister from global client registry
                websocket_client_unregister(wsc);
                
                // Finally free the client structure
                freez(wsc);
                netdata_log_info("WEBSOCKET: Client %zu removed and resources freed", client_id);
                break;
            }

            case WEBSOCKET_THREAD_CMD_BROADCAST: {
                if(header.len < sizeof(size_t) + sizeof(WEBSOCKET_OPCODE)) {
                    netdata_log_error("WEBSOCKET: Invalid broadcast command size");
                    continue;
                }
                
                // Extract broadcast parameters
                size_t offset = 0;
                
                // Extract message length
                size_t message_len;
                memcpy(&message_len, buffer + offset, sizeof(size_t));
                offset += sizeof(size_t);
                
                // Extract opcode
                WEBSOCKET_OPCODE opcode;
                memcpy(&opcode, buffer + offset, sizeof(WEBSOCKET_OPCODE));
                offset += sizeof(WEBSOCKET_OPCODE);
                
                // Ensure we have the complete message
                if(header.len != sizeof(size_t) + sizeof(WEBSOCKET_OPCODE) + message_len) {
                    netdata_log_error("WEBSOCKET: Broadcast command size mismatch");
                    continue;
                }
                
                // Extract message
                const char *message = buffer + offset;
                
                // Send to all clients in this thread
                spinlock_lock(&wth->clients_spinlock);
                
                WEBSOCKET_SERVER_CLIENT *wsc = wth->clients_first;
                while(wsc) {
                    if(wsc->state == WS_STATE_OPEN) {
                        websocket_send_message(wsc, message, message_len, opcode);
                    }
                    wsc = wsc->next;
                }
                
                spinlock_unlock(&wth->clients_spinlock);
                break;
            }

            default:
                netdata_log_error("WEBSOCKET: Unknown command %u", header.cmd);
                break;
        }
    }
}

// Cancel all WebSocket threads
void websocket_threads_cancel(void) {
    for(size_t i = 0; i < WEBSOCKET_MAX_THREADS; i++) {
        if(websocket_threads[i].thread) {
            // Send exit command
            websocket_thread_send_command(&websocket_threads[i], WEBSOCKET_THREAD_CMD_EXIT, NULL, 0);
            
            // Signal thread to cancel
            nd_thread_signal_cancel(websocket_threads[i].thread);
        }
    }

    // Wait for all threads to exit
    for(size_t i = 0; i < WEBSOCKET_MAX_THREADS; i++) {
        if(websocket_threads[i].thread) {
            nd_thread_join(websocket_threads[i].thread);
            websocket_threads[i].thread = NULL;
            websocket_threads[i].running = false;
        }
    }
}

// Get thread by ID
WEBSOCKET_THREAD *websocket_thread_by_id(size_t id) {
    if(id >= WEBSOCKET_MAX_THREADS)
        return NULL;
    return &websocket_threads[id];
}