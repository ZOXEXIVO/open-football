import { Injectable } from "@angular/core";
import { Subject } from "rxjs";
import { MatchDataService } from "./match.data.service";

@Injectable({
    providedIn: 'root',
})
export class MatchPlayService {
    currentState: MatchEvent = MatchEvent.None;

    matchEvents = new Subject<MatchEvent>();
    public matchEvents$ = this.matchEvents.asObservable();

    timeChanged = new Subject<number>();
    public timeChanged$ = this.timeChanged.asObservable();

    currentTime = 0;
    private lastFrameTime = 0;
    private playbackSpeed = 0.7;
    private matchDurationMs: number = 0;

    constructor(private matchDataService: MatchDataService) {
    }

    setMatchDuration(durationMs: number) {
        this.matchDurationMs = durationMs;
    }

    tick(currentTime: number) {
        if (this.currentState === MatchEvent.InProcess) {
            if (this.lastFrameTime === 0) {
                this.lastFrameTime = currentTime;
                return; // Skip first frame to avoid large delta
            }

            const deltaTime = (currentTime - this.lastFrameTime) * this.playbackSpeed;
            this.lastFrameTime = currentTime;

            this.incrementTime(deltaTime);
            this.matchDataService.refreshData(this.currentTime);
        }
    }

    incrementTime(deltaTime: number) {
        this.currentTime += deltaTime;

        // Check if match has ended
        if (this.matchDurationMs > 0 && this.currentTime >= this.matchDurationMs) {
            this.currentTime = this.matchDurationMs;
            this.stop();
            console.log('Match ended at', this.currentTime, 'ms');
        }

        this.timeChanged.next(this.currentTime);
    }

    startMatch() {
        this.currentState = MatchEvent.InProcess;
        this.matchEvents.next(MatchEvent.InProcess);
    }

    pause() {
        this.currentState = MatchEvent.Paused;
        this.matchEvents.next(MatchEvent.Paused);
    }

    stop() {
        this.currentState = MatchEvent.Ended;
        this.matchEvents.next(MatchEvent.Ended);
    }

    reset() {
        this.currentTime = 0;
        this.lastFrameTime = 0;
    }

    setPlaybackSpeed(speed: number) {
        this.playbackSpeed = speed;
    }

    seekToTime(timeMs: number) {
        // Allow restarting from any state except Paused (user explicitly paused)
        const shouldPlay = this.currentState !== MatchEvent.Paused;

        this.currentTime = timeMs;
        this.lastFrameTime = 0; // Reset frame time to continue smooth playback
        this.timeChanged.next(this.currentTime);
        this.matchDataService.refreshData(this.currentTime);

        // Start or maintain playback state after seeking (except when paused)
        if (shouldPlay) {
            this.currentState = MatchEvent.InProcess;
            this.matchEvents.next(MatchEvent.InProcess);
        }
    }
}

export enum MatchEvent {
    None,
    InProcess,
    Paused,
    Ended
}