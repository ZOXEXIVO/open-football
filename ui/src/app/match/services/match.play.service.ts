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

    constructor(private matchDataService: MatchDataService) {
    }

    tick(currentTime: number) {
        if (this.currentState === MatchEvent.InProcess) {
            if (this.lastFrameTime === 0) {
                this.lastFrameTime = currentTime;
            }

            const deltaTime = (currentTime - this.lastFrameTime) * this.playbackSpeed;
            this.lastFrameTime = currentTime;

            this.incrementTime(deltaTime);
            this.matchDataService.refreshData(this.currentTime);
        }
    }

    incrementTime(deltaTime: number) {
        this.currentTime += deltaTime;
        this.timeChanged.next(this.currentTime);
    }

    startMatch() {
        this.currentState = MatchEvent.InProcess;
        this.matchEvents.next(MatchEvent.InProcess);
    }

    pause() {
        this.matchEvents.next(MatchEvent.Paused);
    }

    stop() {
        this.matchEvents.next(MatchEvent.Ended);
    }

    reset() {
        this.currentTime = 0;
        this.lastFrameTime = 0;
    }

    setPlaybackSpeed(speed: number) {
        this.playbackSpeed = speed;
    }
}

export enum MatchEvent {
    None,
    InProcess,
    Paused,
    Ended
}