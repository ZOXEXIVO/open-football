import {
    AfterViewInit,
    ChangeDetectorRef,
    Component,
    ElementRef,
    HostListener,
    Input,
    NgZone,
    OnDestroy,
    OnInit,
    ViewChild
} from '@angular/core';
import * as PIXI from 'pixi.js';
import {Assets, Container, Graphics, TextStyle} from 'pixi.js';
import {UntilDestroy} from "@ngneat/until-destroy";
import {MatchPlayService} from "../services/match.play.service";
import {MatchDataService} from "../services/match.data.service";
import {
    GoalEventDto,
    MatchDataDto,
    MatchMetadataDto,
    MatchPlayerDto,
    MatchService,
    ObjectPositionDto
} from "../services/match.service";

@UntilDestroy()
@Component({
    selector: 'play-match',
    templateUrl: './play.match.component.html',
    standalone: false,
    styleUrls: ['./play.match.component.scss']
})
export class MatchPlayComponent implements AfterViewInit, OnInit, OnDestroy {
    isDisposed = false;

    @ViewChild('matchContainer') matchContainer!: ElementRef;
    @ViewChild('timeScrollbar') timeScrollbar!: ElementRef;
    application: PIXI.Application | null = null;
    dataLoaded = false;
    matchTimeMs: number = -1;
    firstHalfDurationMs: number = -1; // Actual first half duration
    currentTime = 0;
    isFullscreen: boolean = false;
    goals: GoalEventDto[] = [];
    private totalChunks: number = 0;
    private isLoadingChunk: boolean = false;

    // Configuration: Set to true to force 50% half-time position
    // Currently defaulting to true until auto-detection is fixed
    forceEqualHalves: boolean = true;
    @Input()
    leagueSlug: string = '';
    @Input()
    matchId: string = '';
    private background: PIXI.Sprite | null = null;
    private gameContainer: PIXI.Container | null = null;
    private aspectRatio: number = 16 / 10;
    private maxWidth: number = 1400;
    private maxHeight: number = 950;
    private isDraggingSlider = false;

    constructor(private zone: NgZone,
                private cdr: ChangeDetectorRef,
                public matchPlayService: MatchPlayService,
                public matchService: MatchService,
                public matchDataService: MatchDataService) {
        console.log(PIXI.VERSION);
    }

    ngOnInit(): void {
        this.matchPlayService.timeChanged$.subscribe(async time => {
            this.currentTime = time;

            // Load chunk if needed during playback
            // Don't await during playback to avoid stuttering
            if (this.totalChunks > 0) {
                this.loadChunkIfNeeded(time).catch(err => {
                    console.error('Failed to load chunk during playback:', err);
                });
            }

            // Trigger change detection to update slider position
            if (!this.isDraggingSlider) {
                this.cdr.detectChanges();
            }
        });

        document.addEventListener('fullscreenchange', this.onFullscreenChange.bind(this));
        document.addEventListener('webkitfullscreenchange', this.onFullscreenChange.bind(this));
        document.addEventListener('mozfullscreenchange', this.onFullscreenChange.bind(this));
        document.addEventListener('MSFullscreenChange', this.onFullscreenChange.bind(this));
    }

    @HostListener('window:resize', ['$event'])
    onResize(event: Event) {
        this.resizePixiApp();
    }

    resizePixiApp() {
        if (!this.application || !this.background || !this.gameContainer) return;

        const parent = this.matchContainer.nativeElement.parentElement;
        let width = parent.clientWidth;
        let height = parent.clientHeight;

        // Calculate the dimensions while maintaining aspect ratio
        const containerAspectRatio = width / height;

        if (containerAspectRatio > this.aspectRatio) {
            // Container is wider than needed
            width = height * this.aspectRatio;
        } else {
            // Container is taller than needed
            height = width / this.aspectRatio;
        }

        // Update the application size
        this.application.renderer.resize(width, height);

        // Scale the entire stage (including background and game container)
        const scale = Math.min(width / this.maxWidth, height / this.maxHeight);
        this.application.stage.scale.set(scale);

        // Center the stage
        this.application.stage.position.set(
            (width - this.maxWidth * scale) / 2,
            (height - this.maxHeight * scale) / 2
        );

        // Ensure the background covers the entire original size
        this.background.width = this.maxWidth;
        this.background.height = this.maxHeight;

        this.matchDataService.setResolution(this.maxWidth, this.maxHeight);

        // Force a redraw
        this.forceRedraw();
    }

    async setupGraphics(data: MatchDataDto) {
        if (!this.gameContainer) return;

        //create players
        Object.entries(data.players).forEach(([key, value]: [string, ObjectPositionDto[]]) => {
            let player = this.getPlayer(Number(key));

            if (player) {
                const playerObj = this.createPlayer(value[0].position[0], value[0].position[1], player);

                this.matchDataService.setPlayerGraphicsObject(Number(key), playerObj);

                this.gameContainer!.addChild(playerObj);
            }
        });

        // create ball
        const ball = await this.createBall(data);

        this.gameContainer.addChild(ball);

        this.matchDataService.match!.ball.obj = ball;

        this.matchPlayService.startMatch();
    }

    getPlayer(playerId: number): MatchPlayerDto {
        return this.matchDataService.match!.players.find((player) => player.id == playerId)!;
    }

    public ngAfterViewInit(): void {
        this.matchDataService.setResolution(this.maxWidth, this.maxHeight);

        // First, get metadata to know how many chunks we need to load
        this.matchService.metadata(this.leagueSlug, this.matchId).subscribe(metadata => {
            console.log('Match metadata:', metadata);

            // Set total duration upfront so timeline doesn't change
            this.matchTimeMs = metadata.total_duration_ms;
            this.totalChunks = metadata.chunk_count;
            this.matchDataService.setChunkDuration(metadata.chunk_duration_ms);

            // Calculate first half duration upfront
            if (this.forceEqualHalves) {
                this.firstHalfDurationMs = this.matchTimeMs / 2;
            }

            // Always use chunk loading if metadata is available (even for single chunk)
            this.loadChunkedMatchData(metadata.chunk_count);
        }, error => {
            // Fallback to old method if metadata endpoint doesn't exist (for old matches without chunks)
            console.warn('Metadata endpoint not available, falling back to full data load');
            this.loadFullMatchData();
        });
    }

    private loadFullMatchData(): void {
        this.matchService.data(this.leagueSlug, this.matchId).subscribe(async matchData => {
            this.dataLoaded = true;
            this.matchDataService.setMatchData(matchData);
            await this.initializeMatch(matchData);
        });
    }

    private loadChunkedMatchData(chunkCount: number): void {
        // Load only the first chunk immediately
        this.matchService.chunk(this.leagueSlug, this.matchId, 0).subscribe(async firstChunk => {
            console.log('First chunk (0) loaded');
            this.dataLoaded = true;

            this.matchDataService.setMatchData(firstChunk);
            this.matchDataService.mergeMatchData(firstChunk, 0);
            await this.initializeMatch(firstChunk);

            // Other chunks will be loaded on-demand when seeking
        });
    }

    private async initializeMatch(matchData: MatchDataDto): Promise<void> {
        // Load goals from match data service
        if (this.matchDataService.match?.goals) {
            this.goals = this.matchDataService.match.goals;
        }

        // Only calculate duration if not already set (for non-chunked matches)
        if (this.matchTimeMs <= 0 && matchData.ball && matchData.ball.length > 0) {
            const lastBallPosition = matchData.ball[matchData.ball.length - 1];
            this.matchTimeMs = lastBallPosition.timestamp;
            console.log('Match duration from data:', this.matchTimeMs, 'ms');

            // Detect first half duration
            if (this.forceEqualHalves) {
                this.firstHalfDurationMs = this.matchTimeMs / 2;
                console.log('Forcing equal halves - First half duration:', this.firstHalfDurationMs, 'ms');
            } else {
                this.firstHalfDurationMs = this.detectFirstHalfDuration(matchData);
                console.log('Auto-detected first half duration:', this.firstHalfDurationMs, 'ms');
            }
        } else {
            console.log('Using metadata duration:', this.matchTimeMs, 'ms');
        }

        await this.initGraphics();
        await this.setupGraphics(matchData);
    }

    private loadChunkIfNeeded(timestamp: number): Promise<void> {
        const chunkNumber = this.matchDataService.getChunkNumberForTime(timestamp);

        // Check if chunk is already loaded
        if (this.matchDataService.isChunkLoaded(chunkNumber)) {
            return Promise.resolve();
        }

        // Validate chunk number
        if (chunkNumber < 0 || chunkNumber >= this.totalChunks) {
            return Promise.resolve();
        }

        // If already loading this chunk, wait for it
        if (this.isLoadingChunk) {
            return new Promise((resolve) => {
                const checkInterval = setInterval(() => {
                    if (!this.isLoadingChunk) {
                        clearInterval(checkInterval);
                        resolve();
                    }
                }, 50);
            });
        }

        console.log(`Loading chunk ${chunkNumber} for time ${timestamp}ms`);
        this.isLoadingChunk = true;

        return new Promise((resolve, reject) => {
            this.matchService.chunk(this.leagueSlug, this.matchId, chunkNumber).subscribe(
                chunkData => {
                    console.log(`Chunk ${chunkNumber} loaded successfully`);
                    this.matchDataService.mergeMatchData(chunkData, chunkNumber);
                    this.isLoadingChunk = false;

                    // CRITICAL FIX: Refresh display if we're currently viewing a time in this chunk
                    // This ensures ball and players sync properly after chunk loads
                    const currentChunk = this.matchDataService.getChunkNumberForTime(this.currentTime);
                    if (currentChunk === chunkNumber) {
                        console.log(`Refreshing display for current time ${this.currentTime}ms in loaded chunk ${chunkNumber}`);
                        this.matchDataService.refreshData(this.currentTime);
                    }

                    resolve();
                },
                error => {
                    console.error(`Failed to load chunk ${chunkNumber}:`, error);
                    this.isLoadingChunk = false;
                    reject(error);
                }
            );
        });
    }

    forceRedraw() {
        if (!this.application) return;

        // Remove and re-add all children to force a redraw
        const children = [...this.application.stage.children];
        this.application.stage.removeChildren();
        children.forEach(child => this.application!.stage.addChild(child));

        // Render the stage
        this.application!.render();
    }

    initGraphics(): Promise<void> {
        return this.zone.runOutsideAngular(
            async () => {
                this.application = new PIXI.Application();

                await this.application.init({
                    antialias: true,
                    autoDensity: true,
                    resolution: window.devicePixelRatio,
                    width: this.maxWidth,
                    height: this.maxHeight,
                    backgroundColor: 0x283238 // Dark green background color
                });

                this.matchContainer.nativeElement.appendChild(this.application.canvas);

                this.background = await this.createBackground();
                this.application.stage.addChild(this.background);

                this.gameContainer = new PIXI.Container();
                this.application.stage.addChild(this.gameContainer);

                this.resizePixiApp();

                this.application.ticker.add((delta) => {
                    if (this.isDisposed) {
                        return;
                    }

                    const currentTime = performance.now();
                    this.matchPlayService.tick(currentTime);
                });
            }
        );
    }

    createPlayer(x: number, y: number, player: MatchPlayerDto): Container {
        const container = new Container();

        container.position.x = x - 10;
        container.position.y = y - 10;

        const playerColor = this.getColor(player)
        const borderColor = this.getBorderColor(playerColor);

        // Create border circle
        const border = new Graphics();
        border
            .circle(6, 6, 19)  // Slightly larger radius for the border
            .fill(borderColor);

        container.addChild(border);

        // Create player circle
        const circle = new Graphics();
        circle
            .circle(6, 6, 16)
            .fill(playerColor);

        container.addChild(circle);

        const numberStyle = new TextStyle({
            fontFamily: 'Arial, sans-serif',
            fontSize: 14,
            fontWeight: 'bold',
            fill: this.getShirtNumber(player),
            align: 'center'
        });

        const numberText = new PIXI.Text({text: player.shirt_number.toString(), style: numberStyle});

        numberText.anchor.set(0.5);
        numberText.position.set(6, 6); // Center of the circle

        container.addChild(numberText);

        const style = new TextStyle({
            fontFamily: 'Verdana, sans-serif',
            fontSize: 17,
            fill: 'white',
            wordWrap: false,
            align: 'center'
        });

        const text = new PIXI.Text({text: player.last_name, style});

        text.x = 10;
        text.y = 40;

        text.anchor.set(0.5); // Set anchor to center for center alignment

        container.addChild(text);

        return container;
    }

    getColor(player: MatchPlayerDto) {
        if (player.position == "GK") {
            return 0xf7e300;
        }

        const homeColor = 0x00307d;
        const awayColor = 0xb33f00;

        return player.is_home ? homeColor : awayColor;
    }

    getShirtNumber(player: MatchPlayerDto) {
        if (player.position == "GK") {
            return 'black';
        }

        return 'white';
    }


    getBorderColor(color: number): number {
        if (color == 0xf7e300) {
            return 0x000000;
        }

        return 0xffffff;
    }

    async createBackground() {
        const landscapeTexture = await Assets.load('assets/images/match/field.svg');
        const background = new PIXI.Sprite(landscapeTexture);

        background.width = this.maxWidth;
        background.height = this.maxHeight;

        return background;
    }

    async createBall(data: MatchDataDto): Promise<Container> {
        const container = new Container();

        // Create white circle with black border for ball
        const ballCircle = new Graphics();
        const ballRadius = 6;

        // Draw white fill
        ballCircle.circle(0, 0, ballRadius).fill(0xFFFFFF);

        // Draw black border
        ballCircle.circle(0, 0, ballRadius).stroke({width: 2, color: 0x000000});

        container.addChild(ballCircle);

        const z = data.ball[0].position[2] || 0;
        const translatedBallCoods = this.matchDataService.translateToField(
            data.ball[0].position[0], data.ball[0].position[1], z
        );

        container.position.x = translatedBallCoods.x;
        container.position.y = translatedBallCoods.y;

        // Apply initial z-coordinate scaling
        const heightScale = 1.0 + Math.min(z / 15.0, 0.4);
        container.scale.set(heightScale);

        return container;
    }

    toggleFullscreen() {
        if (!this.isFullscreen) {
            this.openFullscreen();
        } else {
            this.closeFullscreen();
        }
    }

    openFullscreen() {
        const elem = this.matchContainer.nativeElement;

        if (elem.requestFullscreen) {
            elem.requestFullscreen();
        } else if (elem.mozRequestFullScreen) { /* Firefox */
            elem.mozRequestFullScreen();
        } else if (elem.webkitRequestFullscreen) { /* Chrome, Safari & Opera */
            elem.webkitRequestFullscreen();
        } else if (elem.msRequestFullscreen) { /* IE/Edge */
            elem.msRequestFullscreen();
        }
    }

    closeFullscreen() {
        if (document.exitFullscreen) {
            document.exitFullscreen();
        } else if ((document as any).mozCancelFullScreen) { /* Firefox */
            (document as any).mozCancelFullScreen();
        } else if ((document as any).webkitExitFullscreen) { /* Chrome, Safari & Opera */
            (document as any).webkitExitFullscreen();
        } else if ((document as any).msExitFullscreen) { /* IE/Edge */
            (document as any).msExitFullscreen();
        }
    }

    onFullscreenChange() {
        const fullscreenElement =
            document.fullscreenElement ||
            (document as any).webkitFullscreenElement ||
            (document as any).mozFullScreenElement ||
            (document as any).msFullscreenElement;

        this.isFullscreen = !!fullscreenElement;

        // Resize the PIXI application
        if (this.application) {
            this.application.resize();
        }
    }

    // Time scrollbar methods
    getSliderPosition(): number {
        if (this.matchTimeMs <= 0) return 0;
        return (this.currentTime / this.matchTimeMs) * 100;
    }

    getGoalPosition(goalTime: number): number {
        if (this.matchTimeMs <= 0) return 0;
        return (goalTime / this.matchTimeMs) * 100;
    }

    isHomeGoal(goal: GoalEventDto): boolean {
        const player = this.matchDataService.match?.players.find(p => p.id === goal.player_id);
        return player?.is_home ?? false;
    }

    formatTime(ms: number): string {
        const totalSeconds = Math.floor(ms / 1000);
        const minutes = Math.floor(totalSeconds / 60);
        const seconds = totalSeconds % 60;
        return `${minutes}:${seconds.toString().padStart(2, '0')}`;
    }

    detectFirstHalfDuration(matchData: MatchDataDto): number {
        if (!matchData.ball || matchData.ball.length === 0) {
            // Fallback: assume equal halves
            console.log('No ball data, using half of match time');
            return this.matchTimeMs / 2;
        }

        console.log('Total ball positions:', matchData.ball.length);
        console.log('Match time range:', matchData.ball[0].timestamp, 'to', matchData.ball[matchData.ball.length - 1].timestamp);

        // Strategy 1: Look for a significant time gap (half-time break)
        // In simulation data, look for any gap > 1 second (unusual in continuous simulation)
        let maxGap = 0;
        let maxGapIndex = -1;
        let maxGapTime = 0;

        for (let i = 1; i < matchData.ball.length; i++) {
            const gap = matchData.ball[i].timestamp - matchData.ball[i - 1].timestamp;

            if (gap > maxGap) {
                maxGap = gap;
                maxGapIndex = i;
                maxGapTime = matchData.ball[i - 1].timestamp;
            }
        }

        console.log('Max gap found:', maxGap, 'ms at index', maxGapIndex, 'time:', maxGapTime);

        // If we found a gap larger than 10 seconds, it's likely a half-time break
        if (maxGap > 10000 && maxGapIndex > 0) {
            const firstHalfEnd = matchData.ball[maxGapIndex - 1].timestamp;
            console.log('Using gap-based detection, first half ends at:', firstHalfEnd);
            return firstHalfEnd;
        }

        // Strategy 2: Look for timestamp around 45 minutes (2,700,000 ms)
        // with tolerance (±10 minutes to handle stoppage time)
        const targetFirstHalf = 2700000; // 45 minutes
        const tolerance = 600000; // 10 minutes

        let closestIndex = -1;
        let closestDiff = Infinity;

        for (let i = 0; i < matchData.ball.length; i++) {
            const timestamp = matchData.ball[i].timestamp;
            const diff = Math.abs(timestamp - targetFirstHalf);

            if (diff < closestDiff && diff < tolerance) {
                closestDiff = diff;
                closestIndex = i;
            }
        }

        if (closestIndex > 0) {
            const firstHalfEnd = matchData.ball[closestIndex].timestamp;
            console.log('Using 45-min proximity detection, first half ends at:', firstHalfEnd, 'diff:', closestDiff);
            return firstHalfEnd;
        }

        // Fallback: Use 50% of match time (equal halves)
        const halfOfMatch = this.matchTimeMs / 2;
        console.log('Using fallback (50% of match time):', halfOfMatch);
        return halfOfMatch;
    }

    getHalfTimePosition(): number {
        // Return percentage position of half-time delimiter
        if (this.matchTimeMs <= 0 || this.firstHalfDurationMs <= 0) {
            console.log('getHalfTimePosition: Using default 50%');
            return 50; // Default to middle
        }
        const position = (this.firstHalfDurationMs / this.matchTimeMs) * 100;
        console.log('getHalfTimePosition:', {
            firstHalfDurationMs: this.firstHalfDurationMs,
            matchTimeMs: this.matchTimeMs,
            position: position.toFixed(2) + '%'
        });
        return position;
    }

    formatMatchTime(ms: number): string {
        const totalSeconds = Math.floor(ms / 1000);
        const totalMinutes = Math.floor(totalSeconds / 60);
        const seconds = totalSeconds % 60;

        if (this.firstHalfDurationMs <= 0) {
            // Fallback to simple time display
            return `${totalMinutes}:${seconds.toString().padStart(2, '0')}`;
        }

        const firstHalfMinutes = Math.floor(this.firstHalfDurationMs / 1000 / 60);

        if (ms < this.firstHalfDurationMs) {
            // First half
            return `${totalMinutes}:${seconds.toString().padStart(2, '0')} (1st)`;
        } else {
            // Second half - show minutes relative to second half
            const secondHalfSeconds = Math.floor((ms - this.firstHalfDurationMs) / 1000);
            const secondHalfMinutes = Math.floor(secondHalfSeconds / 60);
            const secondHalfSecs = secondHalfSeconds % 60;
            return `${secondHalfMinutes}:${secondHalfSecs.toString().padStart(2, '0')} (2nd)`;
        }
    }

    onTimeBarMouseDown(event: MouseEvent): void {
        this.seekToPosition(event);
    }

    onSliderMouseDown(event: MouseEvent): void {
        event.stopPropagation();
        this.isDraggingSlider = true;

        const onMouseMove = (e: MouseEvent) => {
            if (this.isDraggingSlider) {
                this.seekToPosition(e);
            }
        };

        const onMouseUp = () => {
            this.isDraggingSlider = false;
            document.removeEventListener('mousemove', onMouseMove);
            document.removeEventListener('mouseup', onMouseUp);
        };

        document.addEventListener('mousemove', onMouseMove);
        document.addEventListener('mouseup', onMouseUp);
    }

    private async seekToPosition(event: MouseEvent): Promise<void> {
        if (!this.timeScrollbar || this.matchTimeMs <= 0) return;

        const rect = this.timeScrollbar.nativeElement.getBoundingClientRect();
        const x = event.clientX - rect.left;
        const percentage = Math.max(0, Math.min(1, x / rect.width));
        const newTime = percentage * this.matchTimeMs;

        // Load chunk if needed and WAIT for it to complete before seeking
        // This ensures ball and players are synchronized when seeking
        await this.loadChunkIfNeeded(newTime);

        this.matchPlayService.seekToTime(newTime);
    }

    ngOnDestroy(): void {
        this.matchPlayService.reset();
        this.matchDataService.reset();

        this.isDisposed = true;
        this.application?.ticker.stop();

        document.removeEventListener('fullscreenchange', this.onFullscreenChange.bind(this));
        document.removeEventListener('webkitfullscreenchange', this.onFullscreenChange.bind(this));
        document.removeEventListener('mozfullscreenchange', this.onFullscreenChange.bind(this));
        document.removeEventListener('MSFullscreenChange', this.onFullscreenChange.bind(this));
    }
}
